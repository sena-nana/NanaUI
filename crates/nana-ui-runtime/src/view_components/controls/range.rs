#![allow(unused_imports)]
use std::sync::Arc;

use super::super::ComponentView;
use super::super::component_view::{
    TextFieldProjection, apply_input_recipe, compact_icon_style, control_layout,
    format_range_value, project_common, project_text_field, range_field_style, shared_label,
    text_area_footer_extent, text_field_style, tool_box, with_unit,
};
use super::super::events::*;
use crate::layout_edits::{LayoutEdit, LengthBits, StackPreset, edit_layout};
use crate::{
    AccessibilityRole, AccessibilityState, HighlightRequest, InteractionState, MutationQueue,
    NodeKind, NodeStyle, OverlayHostState, ScrollOffset, SemanticPaint, StableNodeId, TextCodeFold,
    TextColorSwatchSpan, TextCompletion, TextContent, TextDiagnosticSpan, TextEditorRenderOptions,
    TextGitMark, TextHorizontalAlignment, TextHover, TextInlay, TextInputState, TextMatchSpan,
    TextSignatureHelp, TextVerticalAlignment, UiWorld,
};
use crate::{AppContext, Entity};

/// A mark on a [`RangeField`] track. `value` uses the range's own units.
/// An empty `label` draws the mark without a name.
#[derive(Debug, Clone, PartialEq)]
pub struct RangeMarker {
    pub value: f64,
    pub label: Arc<str>,
}

impl RangeMarker {
    pub fn new(value: f64, label: impl Into<Arc<str>>) -> Self {
        Self {
            value,
            label: label.into(),
        }
    }
}

/// How close a pointer must be, in logical pixels, before a press or drag
/// lands on a marker instead of the proportional position: the same slop a
/// resize handle gives a pointer near it.
const RANGE_MARKER_SNAP_PX: f32 = nana_ui_core::space::SM;

#[derive(Debug, Clone, PartialEq)]
pub struct RangeField {
    pub value: f64,
    pub minimum: f64,
    pub maximum: f64,
    pub step: f64,
    pub page_step: f64,
    /// Marks drawn on the track. A value outside the range is neither drawn
    /// nor hittable. The volume slider leaves this empty.
    pub markers: Arc<[RangeMarker]>,
    pub label: Option<Arc<str>>,
    pub unit: Option<Arc<str>>,
    /// When false, the current-value (and unit) readout is omitted so the track
    /// can fill the control. Accessibility still exposes the numeric value.
    pub show_value: bool,
    /// When false, [`Self::label`] names the slider for accessibility only and
    /// the track takes the width the drawn label would have used.
    pub show_label: bool,
    /// Spoken name in place of the visible one (`aria-label`): for a control
    /// drawn without text, or several sharing one visible word. `None` keeps
    /// the visible label.
    pub accessible_name: Option<Arc<str>>,
    /// Said after the name by assistive technology (`aria-description`),
    /// never drawn. `None` says nothing more.
    pub accessible_description: Option<Arc<str>>,
    pub size: nana_ui_core::ControlSize,
    /// `Some(girth)` draws only a rail that thick across the whole box; see
    /// [`Self::rail`].
    pub rail: Option<f32>,
    /// `Some(stops)` paints the track as this gradient; see
    /// [`Self::track_gradient`].
    pub track_gradient: Option<Arc<[nana_ui_core::GradientStop]>>,
    /// `Some(value)`: the track from this value to the maximum is a caution
    /// zone; see [`Self::caution_from`].
    pub caution_from: Option<f64>,
    /// Draw the value inside a wide capsule thumb instead of beside the
    /// track; see [`Self::value_thumb`].
    pub value_thumb: bool,
    /// Write the minimum and the maximum at the ends of the track; see
    /// [`Self::show_bounds`].
    pub show_bounds: bool,
    /// `Some(value)` ends the row with a button that puts this value back;
    /// see [`Self::default_value`].
    pub default_value: Option<f64>,
    /// Typing a value: the assembled number field lies over the capsule and
    /// has the focus. Only a value thumb types.
    pub editing: bool,
    pub disabled: bool,
    pub invalid: bool,
    pub dragging: Option<RangeDragState>,
    pub style: NodeStyle,
}

/// The children a range assembles: the reset button and the number field a
/// value thumb types in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RangeParts {
    pub(crate) reset: StableNodeId,
    pub(crate) input: StableNodeId,
}

/// A pointer drag on a range in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RangeDragState {
    pub pointer_id: u64,
    /// The value a cancel restores.
    pub initial_value: f64,
    /// Where the press landed, in the range's layout space.
    pub(crate) press_x: f32,
    /// The pointer's last position.
    pub(crate) last_x: f32,
    /// The unquantized value the pointer reached last.
    pub(crate) last_raw: f64,
    /// `Some((x, value))`: the drag is relative, the value moving from
    /// `value` by the pointer's travel from `x` (a press on a value thumb,
    /// or once Alt has changed the pace). `None`: the pointer's position
    /// picks the value.
    pub(crate) anchor: Option<(f32, f64)>,
    /// Alt held at the last move: the drag moves at a tenth of the pace.
    pub(crate) fine: bool,
    /// The press landed on the value thumb's capsule.
    pub(crate) on_thumb: bool,
    /// The pointer has travelled past the click slop since the press.
    pub(crate) moved: bool,
}

impl RangeDragState {
    /// A drag by `pointer_id` starting at `initial_value`; the pointer's
    /// position picks the value.
    pub fn new(pointer_id: u64, initial_value: f64) -> Self {
        Self {
            pointer_id,
            initial_value,
            press_x: 0.0,
            last_x: 0.0,
            last_raw: initial_value,
            anchor: None,
            fine: false,
            on_thumb: false,
            moved: false,
        }
    }
}

impl RangeField {
    /// Builds a range field, repairing inconsistent bounds instead of failing.
    ///
    /// Non-finite or inverted bounds fall back to `0.0..=1.0`; a non-finite or
    /// non-positive `step` falls back to one hundredth of the span; `value` is
    /// clamped into the resulting range and quantized.
    pub fn new(value: f64, minimum: f64, maximum: f64, step: f64) -> Self {
        let (minimum, maximum) = if minimum.is_finite() && maximum.is_finite() && minimum < maximum
        {
            (minimum, maximum)
        } else {
            (0.0, 1.0)
        };
        let step = if step.is_finite() && step > 0.0 {
            step
        } else {
            (maximum - minimum) / 100.0
        };
        let value = if value.is_finite() {
            value.clamp(minimum, maximum)
        } else {
            minimum
        };
        let mut field = Self {
            value,
            minimum,
            maximum,
            step,
            page_step: step * 10.0,
            markers: Arc::from([]),
            label: None,
            unit: None,
            show_value: true,
            show_label: true,
            accessible_name: None,
            accessible_description: None,
            size: nana_ui_core::ControlSize::Medium,
            rail: None,
            track_gradient: None,
            caution_from: None,
            value_thumb: false,
            show_bounds: false,
            default_value: None,
            editing: false,
            disabled: false,
            invalid: false,
            dragging: None,
            style: range_field_style(),
        };
        field.value = field.quantize(value);
        field
    }

    pub fn markers(mut self, markers: impl Into<Arc<[RangeMarker]>>) -> Self {
        self.markers = markers.into();
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
    pub fn show_label(mut self, show_label: bool) -> Self {
        self.show_label = show_label;
        self
    }

    /// See [`Self::accessible_name`](field@Self::accessible_name); an empty
    /// name is none.
    pub fn accessible_name(mut self, name: impl Into<Arc<str>>) -> Self {
        let name = name.into();
        self.accessible_name = (!name.is_empty()).then_some(name);
        self
    }

    /// See [`Self::accessible_description`](field@Self::accessible_description);
    /// an empty description is none.
    pub fn accessible_description(mut self, description: impl Into<Arc<str>>) -> Self {
        let description = description.into();
        self.accessible_description = (!description.is_empty()).then_some(description);
        self
    }
    pub fn show_value(mut self, show_value: bool) -> Self {
        self.show_value = show_value;
        self
    }
    pub fn size(mut self, size: nana_ui_core::ControlSize) -> Self {
        self.size = size;
        Arc::make_mut(&mut self.style.layout)
            .theme
            .control_typography(size);
        self
    }
    /// Draw only a rail `girth` thick, filled up to the value, across the
    /// whole box: no drawn label, value readout or field padding, and no
    /// thumb unless the range has keyboard focus. The box keeps its own
    /// height, so a thin rail can sit in a taller hit area. The label still
    /// names the slider for assistive technology. A girth that is not finite
    /// and positive keeps the regular look.
    pub fn rail(mut self, girth: f32) -> Self {
        self.rail = (girth.is_finite() && girth > 0.0).then_some(girth);
        self
    }
    /// Paint the track as a gradient through `stops`, from the minimum end
    /// to the maximum end, for a scale whose values are colours (a hue or an
    /// opacity strip). The strip has no fill up to the value and no field
    /// padding, and the thumb is a [`crate::ContrastRing`] so it reads on
    /// any stop. Label, readout, keyboard steps and the slider semantics are
    /// unchanged; pair it with [`Self::show_value`]`(false)` for a bare
    /// strip. Fewer than two stops keep the regular track; the painter reads
    /// the first eight, as it does for any linear gradient. A rail
    /// ([`Self::rail`]) wins over it.
    pub fn track_gradient(mut self, stops: impl Into<Arc<[nana_ui_core::GradientStop]>>) -> Self {
        let stops: Arc<[nana_ui_core::GradientStop]> = stops.into();
        self.track_gradient = (stops.len() >= 2).then_some(stops);
        self
    }
    /// Mark the track from `value` up to the maximum as a caution zone: a
    /// stretch the value may enter but is past what is usual (gain over
    /// 100%). That stretch of the rail and of the fill is painted in the
    /// theme's warning colour; the thumb, the value, the steps and the
    /// slider semantics are unchanged, and nothing stops the value at its
    /// edge (pair it with a marker there for a stop). A value outside the
    /// range marks nothing. A rail ([`Self::rail`]) or a gradient track
    /// ([`Self::track_gradient`]) wins over it.
    pub fn caution_from(mut self, value: f64) -> Self {
        self.caution_from = Some(value);
        self
    }
    /// Draw the value inside the thumb: a wide accent capsule as wide as
    /// the widest value, whose ends stay on the track, over a thin rail
    /// filled up to its centre. No readout is drawn beside the track. A
    /// press on the capsule drags from where the value is instead of
    /// jumping to the pointer; a click on it, or Enter, types a value into
    /// a number field laid over it. A rail ([`Self::rail`]) or a gradient
    /// track ([`Self::track_gradient`]) keeps its own thumb.
    pub fn value_thumb(mut self, value_thumb: bool) -> Self {
        self.value_thumb = value_thumb;
        self
    }
    /// Write the minimum at the start of the track and the maximum, with
    /// the unit, at its end, at the caption size.
    pub fn show_bounds(mut self, show_bounds: bool) -> Self {
        self.show_bounds = show_bounds;
        self
    }
    /// End the row with a compact reset button that sets this value (a
    /// commit, as [`crate::AppContext::set_range_value`]). The button is
    /// disabled while the range holds it. `None` has no button.
    pub fn default_value(mut self, default_value: Option<f64>) -> Self {
        self.default_value = default_value.filter(|value| value.is_finite());
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }
    /// Whether the value is drawn in the thumb: [`Self::value_thumb`], with
    /// neither a rail nor a gradient track taking the thumb's place.
    pub fn draws_value_thumb(&self) -> bool {
        self.value_thumb && self.rail.is_none() && self.track_gradient.is_none()
    }
    /// The range at its default value, the reset button then disabled.
    pub fn at_default(&self) -> bool {
        self.default_value
            .is_none_or(|default| self.quantize(default) == self.value)
    }
    /// Sets the page step, ignoring a non-finite or non-positive value.
    pub fn page_step(mut self, page_step: f64) -> Self {
        if page_step.is_finite() && page_step > 0.0 {
            self.page_step = page_step;
        }
        self
    }
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
    pub fn ratio(&self) -> f32 {
        ((self.value - self.minimum) / (self.maximum - self.minimum)) as f32
    }
    pub fn quantize(&self, value: f64) -> f64 {
        let steps = ((value.clamp(self.minimum, self.maximum) - self.minimum) / self.step).round();
        (self.minimum + steps * self.step).clamp(self.minimum, self.maximum)
    }

    /// The value a pointer at `x` selects. Within [`RANGE_MARKER_SNAP_PX`] of
    /// a marker, that marker wins over the proportional position.
    pub fn value_at(&self, track_x: f32, track_width: f32, x: f32) -> f64 {
        let span = self.maximum - self.minimum;
        let ratio = if track_width > 0.0 {
            ((x - track_x) / track_width).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let proportional = self.minimum + f64::from(ratio) * span;
        let mut best: Option<(f32, f64)> = None;
        if span.is_finite() && span > 0.0 {
            for marker in self.markers.iter() {
                if !marker.value.is_finite()
                    || marker.value < self.minimum
                    || marker.value > self.maximum
                {
                    continue;
                }
                let marker_ratio = ((marker.value - self.minimum) / span) as f32;
                let marker_x = track_x + track_width * marker_ratio;
                let distance = (marker_x - x).abs();
                if distance <= RANGE_MARKER_SNAP_PX
                    && best.is_none_or(|(nearest, _)| distance < nearest)
                {
                    best = Some((distance, marker.value));
                }
            }
        }
        best.map_or(proportional, |(_, value)| value)
    }

    /// Where the caution zone begins along the track, `0.0..1.0`; none
    /// without one, on a rail or a gradient track, or when it would begin
    /// outside the range or at its very end.
    fn caution_ratio(&self) -> Option<f32> {
        if self.rail.is_some() || self.track_gradient.is_some() {
            return None;
        }
        let from = self.caution_from.filter(|from| from.is_finite())?;
        let span = self.maximum - self.minimum;
        if !(span.is_finite() && span > 0.0) || from < self.minimum || from >= self.maximum {
            return None;
        }
        Some(((from - self.minimum) / span) as f32)
    }

    fn marker_ratios(&self) -> Arc<[f32]> {
        let span = self.maximum - self.minimum;
        if !(span.is_finite() && span > 0.0) {
            return Arc::from([]);
        }
        Arc::from(
            self.markers
                .iter()
                .filter_map(|marker| {
                    if !marker.value.is_finite()
                        || marker.value < self.minimum
                        || marker.value > self.maximum
                    {
                        return None;
                    }
                    let ratio = ((marker.value - self.minimum) / span) as f32;
                    (0.0..=1.0).contains(&ratio).then_some(ratio)
                })
                .collect::<Vec<_>>(),
        )
    }
}

static RANGE_POINTER_OPS: crate::framework::PointerOps = crate::framework::PointerOps {
    press: Some(|cx, doc, id, ptr, button, x, y| {
        if button == 0 {
            cx.begin_range_drag_at(doc, ptr, id, x, y)
        } else {
            Ok(false)
        }
    }),
    is_value_range: true,
    end_value_drag: Some(AppContext::end_range_drag_on),
    ..crate::framework::PointerOps::EMPTY
};

static RANGE_A11Y: crate::framework::A11yBehavior = crate::framework::A11yBehavior {
    adjust_range: Some(|cx, id, adj| cx.adjust_range(Entity::from_stable_id(id), adj)),
    set_range_value: Some(|cx, id, val| cx.set_range_value(Entity::from_stable_id(id), val)),
    activate_context_menu_index: None,
    focus_context_menu_index: None,
    activate_select_option: None,
    step: None,
    set_expanded: None,
    set_text: None,
};

impl ComponentView for RangeField {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::RANGE_FIELD.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        capabilities: Some(|caps| {
            caps.add_step_adjuster(AppContext::adjust_focused_range);
            caps.add_edit_ender(AppContext::end_focused_range_edit);
            super::text_input::install_number_input_capabilities(caps);
        }),
        activation: Some(crate::AppContext::activate_range_field),
        assembler: Some(crate::AppContext::assemble_range_field),
        pointer_ops: Some(&RANGE_POINTER_OPS),
        a11y: Some(&RANGE_A11Y),
        slot_projector: Some(Self::project_with_slots),
        ..crate::TypeBehavior::NONE
    };

    fn reconcile(&mut self, mut next: Self) {
        // Typing in progress is the user's.
        next.editing = self.editing && !next.disabled && next.draws_value_thumb();
        *self = next;
    }

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "range-field".into(),
        }
    }

    // Readouts have no ordinary text node to grow their row after a theme install.
    fn wants_metrics_reproject() -> bool {
        true
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        self.project_with_parts(id, world, mutations, false);
    }
}

impl RangeField {
    fn project_with_slots(
        cx: &crate::AppContext,
        id: StableNodeId,
        view: &Self,
        world: &UiWorld,
        mutations: &mut MutationQueue,
    ) {
        view.project_with_parts(
            id,
            world,
            mutations,
            cx.assembled_slot::<RangeParts>(id).is_some(),
        );
    }

    fn project_with_parts(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        mutations: &mut MutationQueue,
        has_parts: bool,
    ) {
        let value = format_range_value(self.value, self.step);
        let rail = self.rail.filter(|girth| girth.is_finite() && *girth > 0.0);
        let track_gradient = self.track_gradient.clone().filter(|_| rail.is_none());
        let value_thumb = self.draws_value_thumb();
        let show_value = self.show_value && rail.is_none() && !value_thumb;
        let visual = RangeVisual {
            label: self
                .label
                .clone()
                .filter(|_| self.show_label && rail.is_none()),
            value: if show_value {
                Arc::clone(&value)
            } else {
                Arc::from("")
            },
            limits: (show_value || value_thumb).then(|| {
                [
                    format_range_value(self.minimum, self.step),
                    format_range_value(self.maximum, self.step),
                ]
            }),
            unit: if show_value { self.unit.clone() } else { None },
            size: self.size,
            ratio: self.ratio(),
            markers: self.marker_ratios(),
            caution: self.caution_ratio(),
            invalid: self.invalid,
            rail,
            track_gradient: track_gradient.clone(),
            value_thumb: value_thumb.then(|| Arc::clone(&value)),
            editing: value_thumb && self.editing,
            bounds: (self.show_bounds && rail.is_none()).then(|| {
                [
                    format_range_value(self.minimum, self.step),
                    with_unit(
                        format_range_value(self.maximum, self.step),
                        self.unit.as_deref(),
                    ),
                ]
            }),
            reset: self.default_value.is_some() && rail.is_none(),
        };
        if world.widget_visual_as::<RangeVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
        if world.text(id).is_some_and(|text| !text.is_empty()) {
            mutations.set_text(
                id,
                TextContent {
                    value: String::new().into(),
                },
            );
        }
        let mut effective_style = self.style.clone();
        {
            let layout = Arc::make_mut(&mut effective_style.layout);
            if layout.width.is_none() {
                layout.width = Some(nana_ui_core::LengthSpec::Fill);
            }
        }
        if effective_style.layout.height.is_none()
            && effective_style.layout.theme.height.is_none()
            && !matches!(
                effective_style.control_height,
                Some(nana_ui_core::ControlHeight::Exact(_))
            )
        {
            let theme = world.node_theme(id);
            let mut resolved = (*effective_style.layout).clone();
            effective_style.layout.theme.apply(&mut resolved, theme);
            let font = resolved.font_size.unwrap_or(theme.typography().body);
            let line_height =
                resolved
                    .line_height
                    .or(Some(nana_ui_core::LineHeightSpec::Absolute(
                        theme.typography().line,
                    )));
            let line = nana_ui_core::text_line_box_height_px(font, line_height);
            let caption_line = nana_ui_core::text_line_box_height_px(
                self.size.caption_size_in(theme.typography()),
                line_height,
            );
            let thumb = if track_gradient.is_some() {
                theme.spacing().xxxl
            } else if value_thumb {
                crate::world::range_value_thumb_height(
                    self.size,
                    theme.typography(),
                    theme.spacing(),
                )
            } else {
                self.size.text_size_in(theme.typography())
            };
            let padding = resolved.resolved_padding();
            let vertical_padding = effective_style
                .control_padding_y
                .map_or(padding.top + padding.bottom, |padding| {
                    padding.resolve(theme.metrics()) * 2.0
                });
            let border = resolved.resolved_border_edges();
            let minimum =
                line.max(caption_line).max(thumb) + vertical_padding + border.top + border.bottom;
            let control_minimum = effective_style
                .control_height
                .map_or(self.size.height_in(theme.metrics()), |height| {
                    height.resolve(theme.metrics())
                });
            let authored_minimum = match resolved.min_height {
                Some(nana_ui_core::LengthSpec::Px(minimum)) => minimum,
                _ => 0.0,
            };
            effective_style.control_height = None;
            let layout = Arc::make_mut(&mut effective_style.layout);
            layout.min_height = Some(nana_ui_core::LengthSpec::Px(
                minimum.max(control_minimum).max(authored_minimum),
            ));
            layout.theme.min_height = None;
        }
        if rail.is_none() && track_gradient.is_none() {
            effective_style.control_padding_x = Some(nana_ui_core::ControlPadding::Field);
        }
        if has_parts {
            // The containing block of the reset button and the number field,
            // which lie over the row's parts.
            Arc::make_mut(&mut effective_style.layout).position =
                nana_ui_core::PositionSpec::Relative;
        }
        if self.invalid {
            effective_style.border = Some(nana_ui_core::SemanticColorRole::Danger);
        }
        effective_style.interaction.disabled.foreground =
            Some(nana_ui_core::SemanticColorRole::Muted);
        project_common(
            id,
            world,
            mutations,
            &effective_style,
            InteractionState {
                pointer_events: !self.disabled,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::Slider,
                label: self.accessible_name.clone().or_else(|| self.label.clone()),
                description: self.accessible_description.clone(),
                value: Some(match &self.unit {
                    Some(unit) => Arc::from(format!("{} {}", value, unit)),
                    None => value,
                }),
                disabled: self.disabled,
                invalid: self.invalid,
                numeric_minimum: Some(self.minimum),
                numeric_maximum: Some(self.maximum),
                numeric_step: Some(self.step),
                numeric_value: Some(self.value),
                ..AccessibilityState::default()
            },
        );
    }
}

/// Slot used by [`RangeVisual`] for its minimum-bound label, past marker, thumb and focus-ring slots.
pub const RANGE_MINIMUM_SLOT: u64 = 1 << 20;
/// Slot used by [`RangeVisual`] for its maximum-bound label.
pub const RANGE_MAXIMUM_SLOT: u64 = RANGE_MINIMUM_SLOT + 1;
/// Slot used by [`RangeVisual`] for its value-thumb capsule label.
pub const RANGE_THUMB_TEXT_SLOT: u64 = RANGE_MINIMUM_SLOT + 2;

/// Page- or node-local derived geometry for a [`RangeVisual`] node.
#[derive(Debug, Clone, PartialEq)]
pub struct RangeGeometry {
    pub label: Option<crate::ComponentTextRegion>,
    pub value: crate::ComponentTextRegion,
    pub unit: Option<crate::ComponentTextRegion>,
    pub track: crate::LayoutBox,
    /// Theme-resolved thickness of the painted rail inside the thumb band.
    pub track_girth: f32,
    pub border_width: f32,
    /// The ring a gradient track's thumb is drawn as; `None` for the regular filled track.
    pub thumb_ring: Option<crate::ContrastRing>,
    /// The minimum and the maximum written at the track's ends.
    pub bounds: Option<[crate::ComponentTextRegion; 2]>,
    /// The capsule a value thumb is drawn as, at the current value.
    pub value_thumb: Option<crate::RangeValueThumb>,
    /// The caution zone and its theme-resolved colours.
    pub caution: Option<crate::RangeCaution>,
}

/// Widget visual projected by [`RangeField`].
#[derive(Debug, Clone, PartialEq)]
pub struct RangeVisual {
    pub label: Option<Arc<str>>,
    pub value: Arc<str>,
    pub limits: Option<[Arc<str>; 2]>,
    pub unit: Option<Arc<str>>,
    pub size: nana_ui_core::ControlSize,
    pub ratio: f32,
    /// Positions along the track, each in `0.0..=1.0`.
    pub markers: Arc<[f32]>,
    /// `Some(ratio)`: the track from there to its end is a caution zone.
    pub caution: Option<f32>,
    pub invalid: bool,
    /// `Some(girth)`, finite and positive: a rail that thick across the whole content box.
    pub rail: Option<f32>,
    /// `Some(stops)`: the track is this gradient with a [`crate::ContrastRing`] thumb.
    pub track_gradient: Option<Arc<[nana_ui_core::GradientStop]>>,
    /// `Some(text)`: the thumb is a wide capsule with the value written in it.
    pub value_thumb: Option<Arc<str>>,
    /// The value is being typed: the capsule makes way for the number field laid over it.
    pub editing: bool,
    /// The formatted minimum and maximum written at the ends of the track.
    pub bounds: Option<[Arc<str>; 2]>,
    /// The row ends with the reset button's square.
    pub reset: bool,
}

impl RangeVisual {
    pub(crate) fn inputs(
        &self,
        id: StableNodeId,
        world: &UiWorld,
    ) -> Option<crate::layout_engine::parts::RangeInputs> {
        if self.rail.is_some() {
            return None;
        }
        let style = world.computed_style(id)?;
        let node_theme = world.node_theme(id);
        let typography = node_theme.typography();
        let measure = world.chrome_text_measure(id);
        let value = self
            .limits
            .as_ref()
            .filter(|_| self.value_thumb.is_none())
            .map(|limits| {
                limits
                    .iter()
                    .map(|limit| measure.tabular_width(limit, style.font_size, style.font_weight))
                    .fold(0.0, f32::max)
            });
        let caption = self.size.caption_size_in(typography);
        Some(crate::layout_engine::parts::RangeInputs {
            label: self
                .label
                .as_ref()
                .map(|label| measure.width(label, style.font_size, style.font_weight)),
            value,
            unit: self
                .unit
                .as_ref()
                .filter(|_| value.is_some())
                .map(|unit| measure.width(unit, caption, Some(typography.regular))),
            bounds: self.bounds.as_ref().map(|bounds| {
                bounds
                    .each_ref()
                    .map(|bound| measure.tabular_width(bound, caption, Some(typography.regular)))
            }),
            trailing: self.reset.then_some(node_theme.spacing().page),
            gap: world.range_gap_for(id, self.size),
        })
    }

    pub(crate) fn geometry_in(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        content: crate::LayoutBox,
        style: &crate::ComputedStyle,
        style_model: nana_ui_core::StyleModelRef,
        secondary: [f32; 4],
    ) -> RangeGeometry {
        let theme = world.node_theme(id);
        let typography = theme.typography();
        let spacing = theme.spacing();
        let text_region = |bounds, content: Arc<str>, muted: bool, size: f32, weight| {
            crate::ComponentTextRegion {
                bounds,
                content: content.into(),
                color: Some(if muted {
                    secondary
                } else {
                    style.color.unwrap_or_else(|| {
                        style_model
                            .color(nana_ui_core::SemanticColorRole::Text)
                            .as_rgba_array()
                    })
                }),
                font_size: size,
                font_weight: weight,
            }
        };
        if let Some(girth) = self.rail {
            let thumb_h = self.size.text_size_in(typography).min(content.height);
            return RangeGeometry {
                border_width: theme.border().hairline,
                label: None,
                value: text_region(
                    crate::LayoutBox {
                        x: content.x + content.width,
                        y: content.y,
                        height: content.height,
                        ..crate::LayoutBox::default()
                    },
                    Arc::clone(&self.value),
                    false,
                    self.size.text_size_in(typography),
                    Some(typography.medium),
                ),
                unit: None,
                track: crate::LayoutBox {
                    y: content.y + (content.height - thumb_h) / 2.0,
                    height: thumb_h,
                    ..content
                },
                track_girth: girth.min(content.height),
                thumb_ring: None,
                bounds: None,
                value_thumb: None,
                caution: None,
            };
        }

        let parts = world.component_content_parts(id);
        let part = |kind| crate::layout_engine::parts::part_bounds(&parts, kind);
        let lane = crate::layout_engine::parts::range_track_lane(
            content,
            &parts,
            world.range_gap_for(id, self.size),
        );
        let thumb = if self.track_gradient.is_some() {
            spacing.xxxl
        } else if self.value_thumb.is_some() {
            crate::world::range_value_thumb_height(self.size, typography, spacing)
        } else {
            self.size.text_size_in(typography)
        }
        .min(content.height);
        let capsule_width = self.value_thumb.as_ref().map(|text| {
            let measure = world.chrome_text_measure(id);
            let text_width = self
                .limits
                .iter()
                .flatten()
                .chain(std::iter::once(text))
                .map(|text| measure.tabular_width(text, style.font_size, style.font_weight))
                .fold(0.0, f32::max);
            (text_width + spacing.md + spacing.md)
                .max(spacing.page + spacing.page)
                .min(lane.width)
        });
        let travel = capsule_width.unwrap_or(thumb);
        let track = crate::LayoutBox {
            x: lane.x + travel / 2.0,
            y: content.y + (content.height - thumb) / 2.0,
            width: (lane.width - travel).max(0.0),
            height: thumb.min(content.height),
        };
        let disabled = world.accessibility(id).is_some_and(|node| node.disabled);
        let caution = self.caution.map(|from| crate::RangeCaution {
            from,
            color: style_model
                .color(nana_ui_core::SemanticColorRole::Warning)
                .as_rgba_array(),
            thumb: !disabled && self.ratio >= from,
        });
        let value_thumb = self
            .value_thumb
            .as_ref()
            .zip(capsule_width)
            .map(|(text, width)| {
                let capsule = crate::LayoutBox {
                    x: track.x + track.width * self.ratio.clamp(0.0, 1.0) - width / 2.0,
                    width,
                    ..track
                };
                let mut text = text_region(
                    capsule,
                    Arc::clone(text),
                    false,
                    style.font_size,
                    style.font_weight,
                );
                if !disabled {
                    let on_accent = style_model
                        .color(nana_ui_core::SemanticColorRole::AccentText)
                        .as_rgba_array();
                    text.color = Some(match caution.filter(|caution| caution.thumb) {
                        Some(crate::RangeCaution { color: warning, .. }) => {
                            let body = style_model
                                .color(nana_ui_core::SemanticColorRole::Text)
                                .as_rgba_array();
                            if contrast(body, warning) > contrast(on_accent, warning) {
                                body
                            } else {
                                on_accent
                            }
                        }
                        None => on_accent,
                    });
                }
                crate::RangeValueThumb {
                    capsule,
                    text,
                    gap: spacing.xxs,
                    editing: self.editing,
                }
            });
        RangeGeometry {
            caution,
            border_width: theme.border().hairline,
            label: self
                .label
                .as_ref()
                .zip(part(crate::LayoutPartKind::Label))
                .map(|(label, bounds)| {
                    text_region(
                        bounds,
                        Arc::clone(label),
                        false,
                        style.font_size,
                        style.font_weight,
                    )
                }),
            value: text_region(
                part(crate::LayoutPartKind::Value).unwrap_or(crate::LayoutBox {
                    x: content.x + content.width,
                    y: content.y,
                    height: content.height,
                    ..crate::LayoutBox::default()
                }),
                Arc::clone(&self.value),
                false,
                style.font_size,
                style.font_weight,
            ),
            unit: self
                .unit
                .as_ref()
                .zip(part(crate::LayoutPartKind::Detail))
                .map(|(unit, bounds)| {
                    text_region(
                        bounds,
                        Arc::clone(unit),
                        true,
                        self.size.caption_size_in(typography),
                        Some(typography.regular),
                    )
                }),
            track,
            track_girth: if self.track_gradient.is_some() {
                spacing.xl
            } else {
                spacing.xs
            }
            .min(track.height),
            thumb_ring: self
                .track_gradient
                .is_some()
                .then(|| crate::ContrastRing::from_effects(theme.effects())),
            bounds: self.bounds.as_ref().and_then(|[minimum, maximum]| {
                let caption = self.size.caption_size_in(typography);
                let regular = Some(typography.regular);
                Some([
                    text_region(
                        part(crate::LayoutPartKind::Minimum)?,
                        Arc::clone(minimum),
                        true,
                        caption,
                        regular,
                    ),
                    text_region(
                        part(crate::LayoutPartKind::Maximum)?,
                        Arc::clone(maximum),
                        true,
                        caption,
                        regular,
                    ),
                ])
            }),
            value_thumb,
        }
    }

    #[doc(hidden)]
    pub fn paint_with_geometry(
        &self,
        geometry: &RangeGeometry,
        bounds_width: f32,
        style: &crate::ComputedStyle,
        focused: bool,
        focus_color: [f32; 4],
        focus_ring: nana_ui_core::FocusRingMetrics,
    ) -> crate::WidgetPaintOutput {
        let mut out = crate::WidgetPaintOutput {
            owns_node_text: true,
            suppress_root_surface: true,
            ..crate::WidgetPaintOutput::new()
        };
        if let Some(bounds) = &geometry.bounds {
            for (slot, (region, alignment)) in
                [RANGE_MINIMUM_SLOT, RANGE_MAXIMUM_SLOT].into_iter().zip(
                    bounds
                        .iter()
                        .zip([TextHorizontalAlignment::End, TextHorizontalAlignment::Start]),
                )
            {
                let mut text =
                    crate::widget_visual::styled_text_region(region, style, alignment, true);
                text.tabular_nums = true;
                out.push(slot, crate::WidgetPaintOp::Text(text));
            }
        }
        if let Some(thumb) = geometry.value_thumb.as_ref().filter(|thumb| !thumb.editing) {
            let mut text = crate::widget_visual::styled_text_region(
                &thumb.text,
                style,
                TextHorizontalAlignment::Center,
                true,
            );
            text.tabular_nums = true;
            out.push(RANGE_THUMB_TEXT_SLOT, crate::WidgetPaintOp::Text(text));
        }
        if let Some(label) = &geometry.label {
            out.push(
                2,
                crate::WidgetPaintOp::Text(crate::widget_visual::styled_text_region(
                    label,
                    style,
                    TextHorizontalAlignment::Start,
                    true,
                )),
            );
        }
        if !geometry.value.content.is_empty() {
            out.push(
                6,
                crate::WidgetPaintOp::Text(crate::widget_visual::styled_text_region(
                    &geometry.value,
                    style,
                    TextHorizontalAlignment::End,
                    true,
                )),
            );
        }
        if let Some(unit) = &geometry.unit {
            out.push(
                7,
                crate::WidgetPaintOp::Text(crate::widget_visual::styled_text_region(
                    unit,
                    style,
                    TextHorizontalAlignment::Start,
                    true,
                )),
            );
        }

        let ratio = self.ratio.clamp(0.0, 1.0);
        let value_thumb = geometry.value_thumb.as_ref();
        let track_band = geometry.track;
        let gradient = self
            .track_gradient
            .as_ref()
            .filter(|_| self.rail.is_none())
            .zip(geometry.thumb_ring);
        let thumb_extent = track_band.height.max(0.0).min(bounds_width);
        let girth = geometry.track_girth.min(track_band.height).max(0.0);
        let border_width = geometry.border_width;
        let rail = crate::LayoutBox {
            x: track_band.x,
            y: track_band.y + (track_band.height - girth) / 2.0,
            width: track_band.width,
            height: girth,
        };
        let rail_radius = girth / 2.0;
        let caution = geometry
            .caution
            .filter(|_| self.rail.is_none() && gradient.is_none());
        let zone_x = caution.map(|caution| rail.x + rail.width * caution.from.clamp(0.0, 1.0));
        let plain_rail = |piece: crate::LayoutBox| match zone_x {
            Some(zone_x) if zone_x < piece.x + piece.width => (
                crate::LayoutBox {
                    width: (zone_x - piece.x).max(0.0),
                    ..piece
                },
                [rail_radius, 0.0, 0.0, rail_radius],
            ),
            _ => (piece, [rail_radius; 4]),
        };
        let border_as_fill_paint = style.border_color.and_then(|color| {
            crate::widget_visual::matching_paint_color(style.paint_colors.border, color)
        });
        if let Some(thumb) = value_thumb {
            let capsule = thumb.capsule;
            let half = capsule.width / 2.0;
            let lane = crate::LayoutBox {
                x: rail.x - half,
                width: rail.width + capsule.width,
                ..rail
            };
            let cut = if thumb.editing { 0.0 } else { thumb.gap };
            let before = crate::LayoutBox {
                width: (capsule.x - cut - lane.x).max(0.0),
                ..lane
            };
            let after_x = capsule.x + capsule.width + cut;
            let after = crate::LayoutBox {
                x: after_x,
                width: (lane.x + lane.width - after_x).max(0.0),
                ..lane
            };
            let (after, after_radius) = plain_rail(after);
            out.push(
                3,
                crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                    rect: after,
                    fill: style.border_color,
                    fill_paint: border_as_fill_paint,
                    corner_radius: after_radius,
                    ..crate::WidgetQuad::default()
                }),
            );
            out.push(
                4,
                crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                    rect: before,
                    fill: style.background,
                    fill_paint: style.paint_colors.background,
                    corner_radius: [rail_radius; 4],
                    ..crate::WidgetQuad::default()
                }),
            );
        } else if let Some((stops, _)) = gradient {
            out.push(
                3,
                crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                    rect: rail,
                    corner_radius: [rail_radius; 4],
                    background_image: Some(nana_ui_core::BackgroundImage::Gradient(
                        nana_ui_core::CssGradient::Linear(nana_ui_core::LinearGradient {
                            angle_deg: 90.0,
                            stops: stops.to_vec(),
                        }),
                    )),
                    ..crate::WidgetQuad::default()
                }),
            );
        } else {
            let (plain, plain_radius) = plain_rail(rail);
            out.push(
                3,
                crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                    rect: plain,
                    fill: style.border_color,
                    fill_paint: border_as_fill_paint,
                    corner_radius: plain_radius,
                    ..crate::WidgetQuad::default()
                }),
            );
            out.push(
                4,
                crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                    rect: crate::LayoutBox {
                        width: rail.width * ratio,
                        ..rail
                    },
                    fill: style.background,
                    fill_paint: style.paint_colors.background,
                    corner_radius: [rail_radius; 4],
                    ..crate::WidgetQuad::default()
                }),
            );
        }
        let caution_slots = if let Some((caution, from)) = caution.zip(zone_x) {
            let (rail_span, fill_end, rail_start) = match value_thumb {
                Some(thumb) => {
                    let capsule = thumb.capsule;
                    let half = capsule.width / 2.0;
                    let cut = if thumb.editing { 0.0 } else { thumb.gap };
                    (
                        (rail.x - half, rail.x + rail.width + half),
                        capsule.x - cut,
                        capsule.x + capsule.width + cut,
                    )
                }
                None => {
                    let at = rail.x + rail.width * ratio;
                    ((rail.x, rail.x + rail.width), at, at)
                }
            };
            let pieces = [
                (5, from.max(rail_start), rail_span.1, true),
                (6, from, fill_end.min(rail_span.1), false),
            ];
            for (slot, start, end, round_end) in pieces {
                if end <= start {
                    continue;
                }
                let round_start = start > from;
                let radius = |round: bool| if round { rail_radius } else { 0.0 };
                out.push(
                    slot,
                    crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                        rect: crate::LayoutBox {
                            x: start,
                            width: end - start,
                            ..rail
                        },
                        fill: Some(caution.color),
                        corner_radius: [
                            radius(round_start),
                            radius(round_end),
                            radius(round_end),
                            radius(round_start),
                        ],
                        ..crate::WidgetQuad::default()
                    }),
                );
            }
            2
        } else {
            0
        };
        let marker_count = self.markers.len() as u64 + caution_slots;
        for (index, marker) in self.markers.iter().copied().enumerate() {
            let marker = marker.clamp(0.0, 1.0);
            let tick = crate::LayoutBox {
                x: track_band.x + track_band.width * marker - girth * 0.375,
                y: rail.y - girth / 2.0,
                width: girth * 0.75,
                height: rail.height + girth,
            };
            let zone_fill = caution
                .filter(|caution| marker >= caution.from)
                .map(|caution| caution.color);
            out.push(
                5 + caution_slots + index as u64,
                crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                    rect: tick,
                    fill: zone_fill.or(style.background),
                    fill_paint: if zone_fill.is_some() {
                        None
                    } else {
                        style.paint_colors.background
                    },
                    border: style.border_color.map(|color| (color, border_width)),
                    border_paint: style.paint_colors.border,
                    corner_radius: [border_width; 4],
                    ..crate::WidgetQuad::default()
                }),
            );
        }
        let thumb_slot = 5 + marker_count;
        let caution_thumb = caution
            .filter(|caution| caution.thumb)
            .map(|caution| caution.color);
        let thumb_paint = if caution_thumb.is_some() {
            None
        } else {
            style.paint_colors.background
        };
        let thumb_rect = match value_thumb {
            Some(thumb) => thumb.capsule,
            None => crate::LayoutBox {
                x: track_band.x + track_band.width * ratio - thumb_extent / 2.0,
                y: track_band.y + (track_band.height - thumb_extent) / 2.0,
                width: thumb_extent,
                height: thumb_extent,
            },
        };
        let focus_slot = if let Some(thumb) = value_thumb {
            if !thumb.editing {
                out.push(
                    thumb_slot,
                    crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                        rect: thumb_rect,
                        fill: caution_thumb.or(style.background),
                        fill_paint: thumb_paint,
                        corner_radius: [thumb_rect.height / 2.0; 4],
                        ..crate::WidgetQuad::default()
                    }),
                );
            }
            thumb_slot + 1
        } else if let Some((_, ring)) = gradient {
            crate::widget_visual::push_contrast_ring_quads(&mut out, thumb_slot, thumb_rect, ring);
            thumb_slot + 3
        } else {
            if self.rail.is_none() || focused {
                out.push(
                    thumb_slot,
                    crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                        rect: thumb_rect,
                        fill: caution_thumb.or(style.background),
                        fill_paint: thumb_paint,
                        border: style.border_color.map(|color| (color, border_width)),
                        border_paint: style.paint_colors.border,
                        corner_radius: [thumb_extent / 2.0; 4],
                        ..crate::WidgetQuad::default()
                    }),
                );
            }
            thumb_slot + 1
        };
        if focused && !value_thumb.is_some_and(|thumb| thumb.editing) {
            let (width, offset) = crate::widget_visual::focus_ring_stroke(focus_ring);
            let ring_width = width * border_width;
            let outset = (offset + width) * border_width;
            let ring = crate::LayoutBox {
                x: thumb_rect.x - outset,
                y: thumb_rect.y - outset,
                width: thumb_rect.width + outset * 2.0,
                height: thumb_rect.height + outset * 2.0,
            };
            out.push(
                focus_slot,
                crate::WidgetPaintOp::Quad(crate::WidgetQuad {
                    rect: ring,
                    border: Some((focus_color, ring_width)),
                    border_paint: crate::widget_visual::matching_paint_color(
                        style.paint_colors.color,
                        focus_color,
                    ),
                    corner_radius: [ring.width.min(ring.height) / 2.0; 4],
                    ..crate::WidgetQuad::default()
                }),
            );
        }
        out
    }
}

impl crate::WidgetVisual for RangeVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>().is_some_and(|rhs| self == rhs)
    }

    fn theme_dependency_mask(&self) -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::COLOR_ANY
            .union(nana_ui_core::ThemeDependencyMask::METRICS)
            .union(nana_ui_core::ThemeDependencyMask::EFFECT)
    }

    fn is_valid(&self) -> bool {
        self.ratio.is_finite()
            && (0.0..=1.0).contains(&self.ratio)
            && self
                .markers
                .iter()
                .all(|marker| marker.is_finite() && (0.0..=1.0).contains(marker))
    }

    fn visual_parts_read_theme(&self) -> bool {
        true
    }

    fn is_document_select_target(&self) -> bool {
        true
    }

    fn content_layout_changed(&self, prev: Option<&dyn std::any::Any>) -> bool {
        prev.and_then(|p| p.downcast_ref::<Self>()).is_none_or(|a| {
            (
                &a.label,
                &a.limits,
                &a.unit,
                a.size,
                a.rail.is_some(),
                a.value_thumb.is_some(),
                &a.bounds,
                a.reset,
            ) != (
                &self.label,
                &self.limits,
                &self.unit,
                self.size,
                self.rail.is_some(),
                self.value_thumb.is_some(),
                &self.bounds,
                self.reset,
            )
        })
    }

    fn clearing_changes_content_layout(&self) -> bool {
        true
    }

    fn intrinsic_chrome_content(
        &self,
        id: StableNodeId,
        world: &UiWorld,
    ) -> crate::WidgetVisualContent {
        crate::WidgetVisualContent {
            width: self.inputs(id, world).map_or(0.0, |inputs| inputs.extent()),
            ..crate::WidgetVisualContent::default()
        }
    }

    fn solve_content_parts(
        &self,
        id: StableNodeId,
        world: &UiWorld,
        _bounds: crate::LayoutBox,
        content: crate::LayoutBox,
        _writing: nana_ui_core::WritingContext,
        _own_frame: bool,
    ) -> Option<crate::layout_engine::parts::ContentParts> {
        Some(
            self.inputs(id, world)
                .map_or(crate::layout_engine::parts::NO_CONTENT_PARTS, |inputs| {
                    crate::layout_engine::parts::range_parts(content, inputs)
                }),
        )
    }

    fn paint(&self, ctx: &crate::WidgetPaintCtx<'_>) -> crate::WidgetPaintOutput {
        let geometry = self.geometry_in(
            ctx.id,
            ctx.world,
            ctx.content,
            ctx.style,
            ctx.style_model,
            ctx.secondary_text_color(),
        );
        let focus_color = ctx
            .style_model
            .color(
                ctx.theme
                    .recipes()
                    .foreground(nana_ui_core::ComponentRecipeId::Range, false),
            )
            .as_rgba_array();
        self.paint_with_geometry(
            &geometry,
            ctx.bounds.width,
            ctx.style,
            ctx.is_focused(),
            focus_color,
            ctx.theme.metrics().focus_ring,
        )
    }
}

/// The WCAG contrast ratio between two sRGB colours, their alpha ignored.
fn contrast(a: [f32; 4], b: [f32; 4]) -> f32 {
    fn luminance(color: [f32; 4]) -> f32 {
        let linear = |channel: f32| {
            if channel <= 0.04045 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color[0]) + 0.7152 * linear(color[1]) + 0.0722 * linear(color[2])
    }
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
