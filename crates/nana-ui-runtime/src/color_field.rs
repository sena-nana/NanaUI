//! Color well plus hex field. The well is the trigger of a `Popover` whose
//! surface, hanging below it like any popover's, is the picker: assembled
//! children (`XYPad` saturation/value, `RangeField` hue). The host never takes
//! a window handle.
//!
//! The picker paints the colours it picks from. The pad is a picture pad
//! (`XYPadSurface::Picture`): the current hue at full saturation and value,
//! white fading out left to right over it and black fading in top to bottom
//! over that, the standard HSV square. The hue slider is a bare strip through
//! the spectrum (`RangeField::track_gradient`). Both mark the value with a
//! contrast ring.

use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, BackgroundImage, ControlSize, CssGradient, FlexDirection, GradientStop, JustifySpec,
    LengthSpec, LinearGradient, SemanticColorRole,
};

use crate::view_components::{
    IconButton, RangeChanged, RangeField, RangeInput, TextChanged, TextInput, project_common,
};
use crate::{
    AccessibilityRole, AccessibilityState, Activate, AppContext, ComponentView, Entity,
    FocusWithinChanged, FrameworkError, InteractionState, MutationQueue, NodeKind, NodeStyle,
    Popover, PopoverToggled, StableNodeId, Stack, TextContent, UiWorld, XYPad, XYPadEvent,
    XYPadSurface, XYPadValue, xy_pad_height,
};

/// The gap between the frame's inner edge and the well, on its top, bottom
/// and start, so the well sits evenly inside the one frame the field draws.
const SWATCH_INSET: f32 = nana_ui_core::space::XXS + nana_ui_core::HAIRLINE;

/// The well's side for a field of `size`: the control height less the
/// frame's border and the inset on both sides.
fn swatch_side(size: ControlSize, metrics: nana_ui_core::ThemeMetrics) -> f32 {
    (size.height_in(metrics) - 2.0 * (nana_ui_core::HAIRLINE + SWATCH_INSET)).max(0.0)
}

/// Committed RGBA in 0..=1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorChanged {
    pub value: [f32; 4],
}

/// Emitted when the user activates the picker's eyedropper button so the host
/// can sample a screen pixel and finish with [`AppContext::complete_color_pick`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorPickRequested;

/// Live RGBA while the picker is dragged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorInput {
    pub value: [f32; 4],
}

/// Compact color field: swatch, hex, and an assembled HSV picker.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorField {
    pub value: [f32; 4],
    pub hue: f32,
    pub sat: f32,
    pub val: f32,
    /// Whether the picker is open. It owns the picker's open state: write this
    /// field, not the assembled `Popover`, which the next assembly overwrites.
    pub opened: bool,
    pub disabled: bool,
    pub invalid: bool,
    pub size: ControlSize,
    /// Accessible name announced for the field.
    /// Accessible name; `None` says the framework's (`color_field.label`).
    pub label: Option<Arc<str>>,
    pub swatch: Option<StableNodeId>,
    pub hex: Option<StableNodeId>,
    pub picker: Option<StableNodeId>,
    pub pad: Option<StableNodeId>,
    pub hue_slider: Option<StableNodeId>,
    pub eyedropper: bool,
    pub eyedropper_button: Option<StableNodeId>,
    pub bottom_row: Option<StableNodeId>,
    pub(crate) sampling: bool,
    pub style: NodeStyle,
    /// Focus is somewhere in the field (the hex text, the well): the frame
    /// draws the focus border for its borderless parts, as `:focus-within`.
    pub(crate) focus_within: bool,
}

impl ColorField {
    /// Replaces the node style wholesale.
    ///
    /// Builders that derive layout from other props (such as `size`) overwrite
    /// only the fields they own, so call those after this one.
    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }
    /// Overrides the accessible name announced for the field.
    pub fn label(mut self, label: impl Into<Arc<str>>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn new(value: [f32; 4]) -> Self {
        let value = sanitize_rgba(value);
        let (hue, sat, val) = rgb_to_hsv(value);
        Self {
            value,
            hue,
            sat,
            val,
            opened: false,
            disabled: false,
            invalid: false,
            size: ControlSize::Medium,
            label: None,
            swatch: None,
            hex: None,
            picker: None,
            pad: None,
            hue_slider: None,
            eyedropper: true,
            eyedropper_button: None,
            bottom_row: None,
            sampling: false,
            style: field_style(ControlSize::Medium),
            focus_within: false,
        }
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

    pub fn eyedropper(mut self, enabled: bool) -> Self {
        self.eyedropper = enabled;
        self
    }

    pub fn invalid(mut self, invalid: bool) -> Self {
        self.invalid = invalid;
        self
    }

    pub fn opened(mut self, opened: bool) -> Self {
        self.opened = opened && !self.disabled;
        self
    }

    pub fn value(mut self, value: [f32; 4]) -> Self {
        self.apply_rgb(value);
        self
    }

    fn apply_rgb(&mut self, value: [f32; 4]) {
        self.value = sanitize_rgba(value);
        let (hue, sat, val) = rgb_to_hsv(self.value);
        self.hue = hue;
        self.sat = sat;
        self.val = val;
    }

    fn apply_hsv(&mut self) {
        self.value = hsv_to_rgb(self.hue, self.sat, self.val, self.value[3]);
    }

    fn effective_style(&self) -> NodeStyle {
        let mut style = self.style.clone();
        // The frame is the field's only box, so it paints the text field's
        // states for the parts inside it.
        if self.disabled {
            style.background = Some(SemanticColorRole::Subtle);
        } else if self.focus_within {
            style.border = Some(SemanticColorRole::FocusBorder);
        }
        if self.invalid {
            style.border = Some(SemanticColorRole::Danger);
        }
        {
            let layout = std::sync::Arc::make_mut(&mut style.layout);
            layout.direction = Some(FlexDirection::Row);
            layout.align_items = AlignSpec::Center;
            layout.justify_content = JustifySpec::Start;
            layout.gap = Some(LengthSpec::Px(0.0));
            layout.width = Some(LengthSpec::Fill);
        }
        style.control_height = Some(nana_ui_core::ControlHeight::Min(self.size));
        style
    }
}

impl ComponentView for ColorField {
    const TYPE_ID: Option<&'static str> = Some(crate::component_descriptors::COLOR_FIELD.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        assembler: Some(crate::AppContext::assemble_color_field),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn reconcile(&mut self, mut next: Self) {
        // The open picker and the HSV cursor track the drag in progress; the
        // committed value is the application's.
        if next.disabled {
            next.opened = false;
            next.sampling = false;
        } else if self.value == next.value {
            next.opened = self.opened;
            next.hue = self.hue;
            next.sat = self.sat;
            next.val = self.val;
            next.sampling = self.sampling;
        } else {
            next.sampling = self.sampling;
        }
        // Child identities are runtime-owned either way.
        next.swatch = self.swatch;
        next.hex = self.hex;
        next.picker = self.picker;
        next.pad = self.pad;
        next.hue_slider = self.hue_slider;
        next.eyedropper_button = self.eyedropper_button;
        next.bottom_row = self.bottom_row;
        next.focus_within = self.focus_within;
        *self = next;
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "color-field".into(),
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
                pointer_events: false,
                focusable: false,
            },
            AccessibilityState {
                role: AccessibilityRole::Generic,
                label: Some(
                    self.label.clone().unwrap_or_else(|| {
                        Arc::clone(&world.framework_strings().color_field_label)
                    }),
                ),
                value: Some(Arc::from(format_hex(self.value))),
                disabled: self.disabled,
                invalid: self.invalid,
                ..AccessibilityState::default()
            },
        );
    }
}

impl AppContext {
    /// Create or refresh the swatch, hex field, and HSV picker for `field`.
    ///
    /// Event handlers are registered only when the children are first created.
    pub fn assemble_color_field(
        &mut self,
        field: Entity<ColorField>,
    ) -> Result<bool, FrameworkError> {
        let document = self
            .world()
            .node(field.stable_id())
            .ok_or(FrameworkError::MissingView(field.stable_id()))?
            .document;
        let snapshot = self.read(field, Clone::clone)?;
        let created = snapshot.hex.is_none();
        let metrics = self.world().theme_metrics();
        let side = swatch_side(snapshot.size, metrics);
        let height = snapshot.size.height_in(metrics);
        let scale = if height > 0.0 { side / height } else { 1.0 };
        let swatch = match snapshot.swatch.filter(|id| self.world().contains(*id)) {
            Some(id) => Entity::<Stack>::from_stable_id(id),
            None => {
                self.create_detached_component(document, swatch_well(snapshot.value, side, scale))?
            }
        };
        let hex = match snapshot.hex.filter(|id| self.world().contains(*id)) {
            Some(id) => Entity::<TextInput>::from_stable_id(id),
            None => self.create_detached_component(document, hex_input(&snapshot))?,
        };
        let pad = match snapshot.pad.filter(|id| self.world().contains(*id)) {
            Some(id) => Entity::<XYPad>::from_stable_id(id),
            None => self.create_detached_component(document, saturation_value_pad(&snapshot))?,
        };
        let hue = match snapshot.hue_slider.filter(|id| self.world().contains(*id)) {
            Some(id) => Entity::<RangeField>::from_stable_id(id),
            None => self.create_detached_component(document, hue_strip(&snapshot))?,
        };
        let show_eyedropper = snapshot.eyedropper && self.world().eyedropper_supported();
        let bottom_row = match snapshot.bottom_row.filter(|id| self.world().contains(*id)) {
            Some(id) => Entity::<Stack>::from_stable_id(id),
            None => {
                self.create_detached_component(document, Stack::bar(nana_ui_core::space::SM))?
            }
        };
        let (eyedropper, eyedropper_created) = match snapshot
            .eyedropper_button
            .filter(|id| self.world().contains(*id))
        {
            Some(id) => (Some(Entity::<IconButton>::from_stable_id(id)), false),
            None if show_eyedropper => {
                let label = Arc::clone(&self.world().framework_strings().color_field_eyedropper);
                let button = self.create_detached_component(
                    document,
                    eyedropper_button(label, snapshot.disabled),
                )?;
                (Some(button), true)
            }
            None => (None, false),
        };
        let picker = match snapshot.picker.filter(|id| self.world().contains(*id)) {
            Some(id) => Entity::<Popover>::from_stable_id(id),
            None => {
                let name = self.color_field_name(&snapshot);
                self.create_detached_component(document, picker_popover(&snapshot, swatch, name))?
            }
        };

        if eyedropper_created && let Some(button) = eyedropper {
            self.observe(button, field, |field, _: &Activate, cx| {
                if field.disabled || !field.eyedropper {
                    return;
                }
                field.opened = false;
                field.sampling = true;
                cx.reassemble();
                cx.emit(ColorPickRequested);
            })?;
        }

        if created {
            self.on(field, |field, event: &FocusWithinChanged, _| {
                field.focus_within = event.focused;
            })?;
            // The well's press, Enter / Space on it, light dismiss and Escape
            // all toggle the popover itself; mirror that so a later reassembly
            // keeps it, and close it again at once on a disabled field.
            self.observe(picker, field, |field, event: &PopoverToggled, cx| {
                field.opened = event.open && !field.disabled;
                cx.reassemble();
            })?;
            self.observe(hex, field, |field, event: &TextChanged, cx| {
                if let Some(value) = parse_hex(&event.value) {
                    field.apply_rgb(value);
                    cx.emit(ColorChanged { value: field.value });
                }
            })?;
            self.observe(pad, field, |field, event: &XYPadEvent, cx| {
                let sample = match event {
                    XYPadEvent::Input(value) | XYPadEvent::Change(value) => *value,
                };
                field.sat = sample.x.clamp(0.0, 1.0);
                field.val = sample.y.clamp(0.0, 1.0);
                field.apply_hsv();
                cx.reassemble();
                match event {
                    XYPadEvent::Input(_) => cx.emit(ColorInput { value: field.value }),
                    XYPadEvent::Change(_) => cx.emit(ColorChanged { value: field.value }),
                }
            })?;
            self.observe(hue, field, |field, event: &RangeInput, cx| {
                field.hue = event.value as f32;
                field.apply_hsv();
                cx.reassemble();
                cx.emit(ColorInput { value: field.value });
            })?;
            self.observe(hue, field, |field, event: &RangeChanged, cx| {
                field.hue = event.value as f32;
                field.apply_hsv();
                cx.reassemble();
                cx.emit(ColorChanged { value: field.value });
            })?;
        }

        self.update_component(swatch, |well, _| {
            *well = swatch_well(snapshot.value, side, scale);
        })?;
        self.update_component(hex, |input, _| {
            // The text is the user's until the color changes: text that
            // already names this color (in any spelling parse_hex takes)
            // stays as typed, so reassembling for the picker does not replace
            // the field's value and, with it, its undo. A different color, or
            // text that names none, shows the canonical hex.
            let next = format_hex(snapshot.value);
            if parse_hex(&input.state.value).map(format_hex).as_ref() != Some(&next) {
                input.state.replace_value(next);
            }
            input.disabled = snapshot.disabled;
            input.invalid = snapshot.invalid;
        })?;
        self.update_component(pad, |pad, _| {
            pad.value = XYPadValue {
                x: snapshot.sat,
                y: snapshot.val,
            };
            pad.disabled = snapshot.disabled;
            paint_saturation_value(&mut pad.style, snapshot.hue);
        })?;
        self.update_component(hue, |range, _| {
            range.value = snapshot.hue as f64;
            range.disabled = snapshot.disabled;
        })?;
        if let Some(button) = eyedropper {
            let label = Arc::clone(&self.world().framework_strings().color_field_eyedropper);
            self.update_component(button, |button, _| {
                let tooltip_open = button.tooltip_open;
                *button = eyedropper_button(label, snapshot.disabled);
                button.tooltip_open = tooltip_open;
            })?;
        }
        let name = self.color_field_name(&snapshot);
        self.update_component(picker, |popover, _| {
            *popover =
                picker_popover(&snapshot, swatch, name).open(snapshot.opened && !snapshot.disabled);
        })?;
        self.update_component(field, |field, _| {
            field.swatch = Some(swatch.stable_id());
            field.hex = Some(hex.stable_id());
            field.picker = Some(picker.stable_id());
            field.pad = Some(pad.stable_id());
            field.hue_slider = Some(hue.stable_id());
            field.eyedropper_button = eyedropper.map(|b| b.stable_id());
            field.bottom_row = Some(bottom_row.stable_id());
        })?;

        if show_eyedropper && let Some(button) = eyedropper {
            self.reconcile_children(
                bottom_row.stable_id(),
                &[button.stable_id(), hue.stable_id()],
            )?;
        } else {
            self.reconcile_children(bottom_row.stable_id(), &[hue.stable_id()])?;
        }
        self.reconcile_children(
            picker.stable_id(),
            &[swatch.stable_id(), pad.stable_id(), bottom_row.stable_id()],
        )?;
        self.append_children(field.stable_id(), &[picker.stable_id(), hex.stable_id()])?;
        Ok(created)
    }

    /// Completes an active screen color pick on `id`, updating RGB while keeping
    /// the field's existing alpha when `rgb` is `Some`, or cancelling sampling
    /// when `None`. Returns `true` if `id` refers to a live `ColorField` whose
    /// state changed.
    pub fn complete_color_pick(&mut self, id: StableNodeId, rgb: Option<[f32; 3]>) -> bool {
        let entity = Entity::<ColorField>::from_stable_id(id);
        let Ok(updated) = self.update_component(entity, |field, cx| {
            let was_sampling = field.sampling;
            field.sampling = false;
            match rgb {
                Some(rgb) => {
                    let clamp = |v: f32| {
                        if v.is_finite() {
                            v.clamp(0.0, 1.0)
                        } else {
                            0.0
                        }
                    };
                    let value = [clamp(rgb[0]), clamp(rgb[1]), clamp(rgb[2]), field.value[3]];
                    field.apply_rgb(value);
                    field.invalid = false;
                    cx.reassemble();
                    cx.emit(ColorInput { value: field.value });
                    cx.emit(ColorChanged { value: field.value });
                    true
                }
                None => was_sampling,
            }
        }) else {
            return false;
        };
        updated
    }

    /// What the field is called: its own label, or the framework's.
    fn color_field_name(&self, field: &ColorField) -> Arc<str> {
        field
            .label
            .clone()
            .unwrap_or_else(|| Arc::clone(&self.world().framework_strings().color_field_label))
    }

    /// Apply a committed color to the field and its assembled children.
    pub fn set_color_field_value(
        &mut self,
        field: Entity<ColorField>,
        value: [f32; 4],
    ) -> Result<bool, FrameworkError> {
        let changed = self.update_component(field, |field, _| {
            let value = sanitize_rgba(value);
            if field.value == value {
                return false;
            }
            field.apply_rgb(value);
            true
        })?;
        if changed {
            self.assemble_color_field(field)?;
        }
        Ok(changed)
    }
}

/// The field's one frame: a text field's fill and border, exactly a control
/// tall, the well inset on three sides and the bare hex text running to the
/// end edge (the text keeps its own end inset).
fn field_style(size: ControlSize) -> NodeStyle {
    let mut style = NodeStyle::default();
    style.background = Some(SemanticColorRole::Background);
    style.border = Some(SemanticColorRole::Border);
    {
        let layout = std::sync::Arc::make_mut(&mut style.layout);
        layout.padding_left = Some(LengthSpec::Px(SWATCH_INSET));
        layout.padding_top = Some(LengthSpec::Px(SWATCH_INSET));
        layout.padding_bottom = Some(LengthSpec::Px(SWATCH_INSET));
        layout.padding_right = Some(LengthSpec::Px(0.0));
        layout.border_width = Some(nana_ui_core::HAIRLINE);
    }
    style.shape = Some(nana_ui_core::ShapeRole::Field);
    style.control_height = Some(nana_ui_core::ControlHeight::Min(size));
    style
}

/// The hex text takes the field's width beside the swatch, and gives it up
/// in a narrow field instead of running past the frame: a text input's
/// natural width is wider than a short hex needs.
///
/// It is bare: no fill, border or corners of its own, and the frame's full
/// inner height, so the field reads as one control. The frame paints its
/// focus and invalid states instead (see [`ColorField::effective_style`]).
fn hex_input(field: &ColorField) -> TextInput {
    let mut input = TextInput::new(format_hex(field.value)).size(field.size);
    input.style.background = None;
    input.style.border = None;
    input.style.clear_corners();
    input.style.control_height = None;
    input.style.interaction = crate::InteractionStyle {
        disabled: crate::SemanticPaint {
            foreground: Some(SemanticColorRole::Faint),
            ..crate::SemanticPaint::default()
        },
        ..crate::InteractionStyle::default()
    }
    .into();
    // Its own invalid border is the frame's now.
    input.style_override = true;
    let layout = std::sync::Arc::make_mut(&mut input.style.layout);
    layout.border_width = Some(0.0);
    layout.theme.border_width = None;
    layout.align_self = Some(AlignSpec::Stretch);
    layout.min_height = None;
    layout.flex_grow = Some(1.0);
    layout.flex_shrink = Some(1.0);
    layout.min_width = Some(LengthSpec::Px(0.0));
    input
}

/// The picker: a popover whose trigger is the well, named like the field,
/// with a bare trigger so the well sits on the field's own fill. Its surface
/// hangs below the well, start-aligned, and keeps to the window.
fn picker_popover(field: &ColorField, well: Entity<Stack>, name: Arc<str>) -> Popover {
    let mut popover = Popover::new()
        .trigger(name)
        .trigger_content(well.stable_id())
        .bare_trigger(true)
        .compact_trigger(true)
        .alignment(crate::PopoverAlignment::Start)
        .width(220.0);
    popover.open = field.opened;
    popover
}

/// The saturation/value square: saturation across, value up, painted in the
/// field's hue (see [`paint_saturation_value`]).
fn saturation_value_pad(field: &ColorField) -> XYPad {
    let mut pad = XYPad::new(XYPadValue {
        x: field.sat,
        y: field.val,
    })
    .x_range(0.0, 1.0)
    .y_range(0.0, 1.0)
    .surface(XYPadSurface::Picture)
    .height(xy_pad_height(ControlSize::Large) * 2.0);
    paint_saturation_value(&mut pad.style, field.hue);
    pad
}

/// Paint `style` as the HSV square for `hue`: the hue at full saturation and
/// value as the fill, white fading out to the right over it (saturation), and
/// black fading in downwards over that (value). The layers are a node's
/// ordinary background image layers, so the box painter draws them.
fn paint_saturation_value(style: &mut NodeStyle, hue: f32) {
    let layout = Arc::make_mut(&mut style.layout);
    let hue_color = hsv_to_rgb(hue, 1.0, 1.0, 1.0);
    if layout.background == Some(hue_color) && layout.paint.background_image.is_some() {
        return;
    }
    let white = hsv_to_rgb(0.0, 0.0, 1.0, 1.0);
    let black = hsv_to_rgb(0.0, 0.0, 0.0, 1.0);
    let clear = |color: [f32; 4]| [color[0], color[1], color[2], 0.0];
    layout.background = Some(hue_color);
    // CSS order: the first image is on top. Value over saturation.
    layout.paint.background_image = Some(linear_gradient(180.0, clear(black), black));
    layout.paint.background_layers = vec![linear_gradient(90.0, white, clear(white))];
}

fn linear_gradient(angle_deg: f32, from: [f32; 4], to: [f32; 4]) -> BackgroundImage {
    BackgroundImage::Gradient(CssGradient::Linear(LinearGradient {
        angle_deg,
        stops: vec![gradient_stop(0.0, from), gradient_stop(1.0, to)],
    }))
}

fn gradient_stop(position: f32, color: [f32; 4]) -> GradientStop {
    GradientStop {
        paint_color: None,
        position,
        color,
    }
}

/// The screen eyedropper trigger beside the hue strip.
fn eyedropper_button(label: Arc<str>, disabled: bool) -> IconButton {
    IconButton::tool(nana_ui_core::Icon::ColorPicker, label).disabled(disabled)
}

/// The hue slider: a bare spectrum strip from 0 to 360 degrees, no readout.
fn hue_strip(field: &ColorField) -> RangeField {
    let mut range = RangeField::new(field.hue as f64, 0.0, 360.0, 1.0)
        .show_value(false)
        .track_gradient(hue_spectrum());
    let layout = Arc::make_mut(&mut range.style.layout);
    layout.width = Some(LengthSpec::Fill);
    layout.flex_grow = Some(1.0);
    layout.flex_shrink = Some(1.0);
    layout.min_width = Some(LengthSpec::Px(0.0));
    layout.allow_shrink = true;
    range
}

/// The hue circle unrolled, 0 to 360 degrees: red, yellow, green, cyan,
/// blue, magenta and back to red, as gradient stops for a strip such as
/// [`RangeField::track_gradient`].
pub fn hue_spectrum() -> Arc<[GradientStop]> {
    const SEXTANTS: u8 = 6;
    (0..=SEXTANTS)
        .map(|index| {
            let position = f32::from(index) / f32::from(SEXTANTS);
            gradient_stop(position, hsv_to_rgb(position * 360.0, 1.0, 1.0, 1.0))
        })
        .collect()
}

/// The color itself: a square well `side` across, display only (the press is
/// the picker's, which it draws the trigger of). Its corners are the frame's
/// scaled by `scale`, the well's side over the frame's height.
fn swatch_well(value: [f32; 4], side: f32, scale: f32) -> Stack {
    let mut style = NodeStyle::default();
    {
        let layout = std::sync::Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Row);
        layout.width = Some(LengthSpec::Px(side));
        layout.height = Some(LengthSpec::Px(side));
        layout.min_width = Some(LengthSpec::Px(side));
        layout.min_height = Some(LengthSpec::Px(side));
        layout.flex_grow = Some(0.0);
        layout.flex_shrink = Some(0.0);
        layout.border_width = Some(nana_ui_core::HAIRLINE);
        layout.background = Some(sanitize_rgba(value));
        // The frame's corner scaled down to the well, so the well is the
        // field's own shape in small: the same tier, the same proportion,
        // and it follows the installed theme's radius.
        layout.theme.border_radius = Some(
            nana_ui_core::ThemeExpr::token(nana_ui_core::ThemeToken::Radius(
                nana_ui_core::RadiusTier::Sm,
            ))
            .scaled(scale),
        );
    }
    style.border = Some(SemanticColorRole::Border);
    Stack::row(0.0).style(style)
}

pub fn sanitize_rgba(value: [f32; 4]) -> [f32; 4] {
    [
        finite_unit(value[0]),
        finite_unit(value[1]),
        finite_unit(value[2]),
        finite_unit(value[3]),
    ]
}

fn finite_unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

pub fn format_hex(value: [f32; 4]) -> String {
    let value = sanitize_rgba(value);
    let r = (value[0] * 255.0 + 0.5) as u8;
    let g = (value[1] * 255.0 + 0.5) as u8;
    let b = (value[2] * 255.0 + 0.5) as u8;
    let a = (value[3] * 255.0 + 0.5) as u8;
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

pub fn parse_hex(raw: &str) -> Option<[f32; 4]> {
    let trimmed = raw.trim();
    let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);
    let bytes = match hex.len() {
        6 => hex_bytes(hex, 3)?,
        8 => hex_bytes(hex, 4)?,
        _ => return None,
    };
    Some([
        bytes[0] as f32 / 255.0,
        bytes[1] as f32 / 255.0,
        bytes[2] as f32 / 255.0,
        bytes.get(3).copied().unwrap_or(255) as f32 / 255.0,
    ])
}

fn hex_bytes(hex: &str, count: usize) -> Option<Vec<u8>> {
    if hex.len() != count * 2 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..count)
        .map(|index| u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok())
        .collect()
}

pub fn rgb_to_hsv(value: [f32; 4]) -> (f32, f32, f32) {
    let [r, g, b, _] = sanitize_rgba(value);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let hue = if delta <= f32::EPSILON {
        0.0
    } else if (max - r).abs() <= f32::EPSILON {
        60.0 * (((g - b) / delta) % 6.0)
    } else if (max - g).abs() <= f32::EPSILON {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };
    let hue = if hue < 0.0 { hue + 360.0 } else { hue };
    let sat = if max <= f32::EPSILON {
        0.0
    } else {
        delta / max
    };
    (hue, sat, max)
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32, a: f32) -> [f32; 4] {
    let h = if h.is_finite() {
        ((h % 360.0) + 360.0) % 360.0
    } else {
        0.0
    };
    let s = finite_unit(s);
    let v = finite_unit(v);
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0).floor() as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    sanitize_rgba([r + m, g + m, b + m, a])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentId;
    use crate::framework::AppContext;

    #[test]
    fn the_frame_draws_the_focus_border_while_the_hex_text_has_focus() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let field = context
            .create_component(document, ColorField::new([1.0, 0.0, 0.0, 1.0]))
            .unwrap();
        context.assemble_color_field(field).unwrap();
        let hex = context.read(field, |field| field.hex).unwrap().unwrap();
        let border = |context: &AppContext| {
            context
                .world()
                .node_style(field.stable_id())
                .unwrap()
                .border
        };
        assert_eq!(border(&context), Some(SemanticColorRole::Border));
        assert!(context.focus_node(document, hex).unwrap());
        assert_eq!(border(&context), Some(SemanticColorRole::FocusBorder));
    }

    #[test]
    fn hex_round_trips_opaque_and_alpha() {
        assert_eq!(parse_hex("#ff8800"), Some([1.0, 136.0 / 255.0, 0.0, 1.0]));
        assert_eq!(format_hex([1.0, 0.0, 0.0, 1.0]), "#ff0000");
        assert_eq!(format_hex([1.0, 0.0, 0.0, 0.5]).len(), 9);
        assert!(parse_hex("nope").is_none());
    }

    #[test]
    fn hsv_preserves_primary_red() {
        let rgb = hsv_to_rgb(0.0, 1.0, 1.0, 1.0);
        assert!((rgb[0] - 1.0).abs() < 1e-5);
        assert!(rgb[1].abs() < 1e-5);
        assert!(rgb[2].abs() < 1e-5);
        let (h, s, v) = rgb_to_hsv(rgb);
        assert!((s - 1.0).abs() < 1e-5);
        assert!((v - 1.0).abs() < 1e-5);
        let _ = h;
    }

    /// A field narrower than the hex text's natural width keeps the hex
    /// input inside its frame: the input gives up width, the swatch keeps
    /// its square.
    #[test]
    fn a_narrow_field_keeps_the_hex_input_inside_its_frame() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let root = context
            .create_component(
                document,
                crate::Stack::row(0.0).width(nana_ui_core::LengthSpec::Px(148.0)),
            )
            .unwrap();
        let field = context
            .create_component(document, ColorField::new([1.0, 1.0, 1.0, 1.0]))
            .unwrap();
        context.append_child(root, field).unwrap();
        assert!(context.assemble_color_field(field).unwrap());
        context
            .layout_document(document, crate::LayoutViewport::new(400.0, 200.0))
            .unwrap();
        let snapshot = context.read(field, Clone::clone).unwrap();
        let world = context.world();
        let frame = world.layout_box(field.stable_id()).unwrap();
        let swatch = world.layout_box(snapshot.swatch.unwrap()).unwrap();
        let hex = world.layout_box(snapshot.hex.unwrap()).unwrap();
        assert!(
            hex.x + hex.width <= frame.x + frame.width + 0.5,
            "hex {hex:?} runs past the field {frame:?}"
        );
        assert!(
            (swatch.width - swatch_side(ControlSize::Medium, world.theme_metrics())).abs() < 0.5,
            "swatch {swatch:?}"
        );
        // One frame, exactly a control tall: the well sits the same inset
        // from its top, bottom and start, and the bare hex text fills the
        // inner height instead of drawing a second box inside it.
        let edge = nana_ui_core::HAIRLINE + SWATCH_INSET;
        assert!(
            (frame.height - ControlSize::Medium.height_in(world.theme_metrics())).abs() < 0.5,
            "frame {frame:?}"
        );
        assert!(
            (swatch.x - frame.x - edge).abs() < 0.5,
            "start inset {swatch:?}"
        );
        assert!(
            (swatch.y - frame.y - edge).abs() < 0.5,
            "top inset {swatch:?}"
        );
        assert!(
            (frame.y + frame.height - swatch.y - swatch.height - edge).abs() < 0.5,
            "bottom inset {swatch:?}"
        );
        assert!(
            hex.y >= frame.y && hex.y + hex.height <= frame.y + frame.height + 0.5,
            "hex {hex:?} inside {frame:?}"
        );
        let hex_style = world.node_style(snapshot.hex.unwrap()).unwrap();
        assert_eq!((hex_style.background, hex_style.border), (None, None));
        assert!(hex.width > 40.0, "the hex input still reads, {hex:?}");
        // The well is the picker's trigger, first in the field; the closed
        // picker's surface is not laid out in the row, so neither its sliders
        // nor a pixel of them show at the field's edge.
        let picker = world.layout_box(snapshot.picker.unwrap()).unwrap();
        assert!(picker.x <= swatch.x && swatch.x + swatch.width <= picker.x + picker.width);
        assert!(picker.x + picker.width <= hex.x);
        for item in [snapshot.pad.unwrap(), snapshot.hue_slider.unwrap()] {
            let shown = world.is_overlay_reachable(item)
                && world
                    .layout_box(item)
                    .is_some_and(|b| b.width > 0.0 && b.height > 0.0);
            assert!(!shown, "the closed picker's items wait for its surface");
        }
        // Open, the picker hangs below the well instead of joining the row.
        assert!(context.activate_node(snapshot.picker.unwrap()).unwrap());
        context
            .layout_document(document, crate::LayoutViewport::new(400.0, 400.0))
            .unwrap();
        let world = context.world();
        let hue = world.layout_box(snapshot.hue_slider.unwrap()).unwrap();
        let well = world.layout_box(snapshot.swatch.unwrap()).unwrap();
        assert!(hue.y >= well.y + well.height, "{hue:?} under {well:?}");
        let frame_after = world.layout_box(field.stable_id()).unwrap();
        assert_eq!(frame_after, frame, "opening does not reflow the field");
    }

    #[test]
    fn assemble_creates_swatch_hex_and_picker() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let field = context
            .create_component(document, ColorField::new([1.0, 0.0, 0.0, 1.0]))
            .unwrap();
        assert!(context.assemble_color_field(field).unwrap());
        let snapshot = context.read(field, Clone::clone).unwrap();
        assert!(snapshot.swatch.is_some());
        assert!(snapshot.hex.is_some());
        assert!(snapshot.picker.is_some());
        assert!(snapshot.pad.is_some());
        assert!(snapshot.hue_slider.is_some());
        assert!(!context.assemble_color_field(field).unwrap());
    }

    #[test]
    fn the_hue_spectrum_runs_round_the_circle() {
        let stops = hue_spectrum();
        assert_eq!(stops.len(), 7);
        let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5);
        assert!(near(stops[0].color, [1.0, 0.0, 0.0, 1.0]), "red");
        assert!(near(stops[2].color, [0.0, 1.0, 0.0, 1.0]), "green");
        assert!(near(stops[3].color, [0.0, 1.0, 1.0, 1.0]), "cyan");
        assert!(near(stops[4].color, [0.0, 0.0, 1.0, 1.0]), "blue");
        assert!(near(stops[6].color, [1.0, 0.0, 0.0, 1.0]), "back to red");
        assert_eq!((stops[0].position, stops[6].position), (0.0, 1.0));
    }

    /// The picker paints what it picks from: the pad is the HSV square in
    /// the current hue, the hue slider a bare spectrum strip.
    #[test]
    fn the_picker_paints_the_hsv_square_and_a_spectrum_strip() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let field = context
            .create_component(document, ColorField::new([1.0, 0.0, 0.0, 1.0]))
            .unwrap();
        context.assemble_color_field(field).unwrap();
        let snapshot = context.read(field, Clone::clone).unwrap();
        let pad = Entity::<XYPad>::from_stable_id(snapshot.pad.unwrap());
        let hue = Entity::<RangeField>::from_stable_id(snapshot.hue_slider.unwrap());

        let pad_paint = |context: &AppContext| {
            let style = context.world().node_style(pad.stable_id()).unwrap();
            (
                style.layout.background,
                style.layout.paint.background_image.clone(),
                style.layout.paint.background_layers.clone(),
            )
        };
        let (fill, value, saturation) = pad_paint(&context);
        assert_eq!(fill, Some([1.0, 0.0, 0.0, 1.0]), "the hue at full strength");
        assert_eq!(
            value,
            Some(linear_gradient(
                180.0,
                [0.0, 0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 1.0]
            )),
            "black fades in downwards, on top"
        );
        assert_eq!(
            saturation,
            vec![linear_gradient(
                90.0,
                [1.0, 1.0, 1.0, 1.0],
                [1.0, 1.0, 1.0, 0.0]
            )],
            "white fades out to the right, underneath"
        );
        assert_eq!(
            context.read(pad, |pad| (pad.surface, pad.height)).unwrap(),
            (
                XYPadSurface::Picture,
                Some(xy_pad_height(ControlSize::Large) * 2.0)
            )
        );
        assert!(matches!(
            context
                .world()
                .widget_visual_as::<crate::XYPadVisual>(pad.stable_id()),
            Some(crate::XYPadVisual { picture: true, .. })
        ));
        let (show_value, gradient) = context
            .read(hue, |range| {
                (range.show_value, range.track_gradient.clone())
            })
            .unwrap();
        assert!(!show_value, "no numeric readout");
        assert_eq!(gradient, Some(hue_spectrum()));
        assert!(matches!(
            context
                .world()
                .widget_visual_as::<crate::RangeVisual>(hue.stable_id()),
            Some(crate::RangeVisual {
                value,
                track_gradient: Some(_),
                ..
            }) if value.is_empty()
        ));

        // A new hue repaints the square; saturation and value stay put.
        assert!(context.set_range_value(hue, 240.0).unwrap());
        let (fill, value_after, saturation_after) = pad_paint(&context);
        assert_eq!(fill, Some(hsv_to_rgb(240.0, 1.0, 1.0, 1.0)));
        assert_eq!((value_after, saturation_after), (value, saturation));
        let sv = context.read(pad, |pad| pad.value).unwrap();
        assert_eq!(sv, XYPadValue { x: 1.0, y: 1.0 });
    }

    #[test]
    fn the_swatch_opens_the_picker_and_a_dismissal_stays_closed() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let field = context
            .create_component(document, ColorField::new([1.0, 0.0, 0.0, 1.0]))
            .unwrap();
        context.assemble_color_field(field).unwrap();
        let snapshot = context.read(field, Clone::clone).unwrap();
        let picker = Entity::<Popover>::from_stable_id(snapshot.picker.unwrap());
        let hue = Entity::<RangeField>::from_stable_id(snapshot.hue_slider.unwrap());
        let open = |context: &AppContext| context.read(picker, |popover| popover.open).unwrap();

        assert!(context.activate_node(picker.stable_id()).unwrap());
        assert!(open(&context), "the swatch opens the picker");
        assert!(context.read(field, |field| field.opened).unwrap());

        assert!(context.dismiss_popovers_outside(None).unwrap());
        assert!(!context.read(field, |field| field.opened).unwrap());
        context.set_range_value(hue, 200.0).unwrap();
        assert!(
            !open(&context),
            "a picker change must not reopen a dismissed picker"
        );
    }

    #[test]
    fn hex_text_the_user_typed_survives_a_reassembly_of_the_same_color() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let field = context
            .create_component(document, ColorField::new([1.0, 0.0, 0.0, 1.0]))
            .unwrap();
        context.assemble_color_field(field).unwrap();
        let snapshot = context.read(field, Clone::clone).unwrap();
        let hex = snapshot.hex.unwrap();
        let picker = snapshot.picker.unwrap();
        context.focus_node(document, hex).unwrap();
        context.select_all_focused_text(document).unwrap();
        context.replace_focused_text(document, "#00FF88").unwrap();
        // Opening the picker reassembles; the text already names the color.
        assert!(context.activate_node(picker).unwrap());
        let text = |context: &AppContext| {
            context
                .read(Entity::<TextInput>::from_stable_id(hex), |input| {
                    input.state.value.to_string()
                })
                .unwrap()
        };
        assert_eq!(text(&context), "#00FF88", "not rewritten to change case");
        assert!(context.can_undo_text(hex), "so the typing stays undoable");
        // Any spelling of the same color is the user's; a draft that names
        // no color is not.
        context.select_all_focused_text(document).unwrap();
        context.replace_focused_text(document, " 00ff88 ").unwrap();
        assert!(context.activate_node(picker).unwrap());
        assert_eq!(text(&context), " 00ff88 ");
        context.select_all_focused_text(document).unwrap();
        context.replace_focused_text(document, "#00ff8").unwrap();
        assert!(context.activate_node(picker).unwrap());
        assert_eq!(text(&context), "#00ff88");
        // A different color does rewrite it.
        let hue = Entity::<RangeField>::from_stable_id(snapshot.hue_slider.unwrap());
        context.set_range_value(hue, 200.0).unwrap();
        assert_ne!(text(&context).to_ascii_lowercase(), "#00ff88");
    }

    #[test]
    fn a_hue_commit_reports_a_committed_color() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let field = context
            .create_component(document, ColorField::new([1.0, 0.0, 0.0, 1.0]))
            .unwrap();
        context.assemble_color_field(field).unwrap();
        let hue = context
            .read(field, |field| field.hue_slider)
            .unwrap()
            .map(Entity::<RangeField>::from_stable_id)
            .unwrap();
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let input = std::sync::Arc::clone(&events);
        context
            .on(field, move |_, event: &ColorInput, _| {
                input.lock().unwrap().push(("input", event.value));
            })
            .unwrap();
        let changed = std::sync::Arc::clone(&events);
        context
            .on(field, move |_, event: &ColorChanged, _| {
                changed.lock().unwrap().push(("changed", event.value));
            })
            .unwrap();

        assert!(context.set_range_value(hue, 120.0).unwrap());
        let events = events.lock().unwrap();
        let kinds: Vec<_> = events.iter().map(|(kind, _)| *kind).collect();
        assert_eq!(kinds, ["input", "changed"]);
        let green = events[1].1;
        assert!(
            green[1] > 0.99 && green[0] < 0.01,
            "hue 120 is green: {green:?}"
        );
        drop(events);
        let hex = context
            .read(field, |field| field.hex)
            .unwrap()
            .map(Entity::<TextInput>::from_stable_id)
            .unwrap();
        assert_eq!(
            context
                .read(hex, |input| input.state.value.to_string())
                .unwrap(),
            "#00ff00",
            "a picker change refreshes the hex field without a host write"
        );

        assert!(context.focus_node(document, hex.stable_id()).unwrap());
        assert!(context.select_all_focused_text(document).unwrap());
        let mut typed = String::new();
        for key in "#00FF88".chars() {
            typed.push(key);
            context
                .replace_focused_text(document, &key.to_string())
                .unwrap();
            assert_eq!(
                context
                    .read(hex, |input| input.state.value.to_string())
                    .unwrap(),
                typed,
                "typing into the hex field is never rewritten"
            );
        }
        let value = context.read(field, |field| field.value).unwrap();
        assert!(value[2] > 0.5, "a complete hex still commits: {value:?}");
    }

    /// A field built by a view gets its swatch and hex field without the
    /// view assembling it.
    #[test]
    fn a_view_field_assembles_itself() {
        use crate::view::widget;
        let document = crate::DocumentId::new(1).unwrap();
        let mut context = AppContext::new();
        let root = context
            .create_component(document, crate::Stack::column(0.0))
            .unwrap();
        context
            .mount_view(root.stable_id(), || {
                widget(ColorField::new([0.2, 0.4, 0.8, 1.0]))
            })
            .unwrap();
        let field = context.world().node(root.stable_id()).unwrap().children[0];
        let (swatch, hex) = context
            .read(Entity::<ColorField>::from_stable_id(field), |field| {
                (field.swatch, field.hex)
            })
            .unwrap();
        assert!(swatch.is_some() && hex.is_some());
    }

    #[test]
    fn eyedropper_appears_only_when_supported_and_enabled_and_shares_bottom_row() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let root = context
            .create_component(document, crate::Stack::column(nana_ui_core::space::SM))
            .unwrap();
        let supported = context
            .create_component(document, ColorField::new([1.0, 0.0, 0.0, 1.0]))
            .unwrap();
        let opted_out = context
            .create_component(
                document,
                ColorField::new([0.0, 1.0, 0.0, 1.0]).eyedropper(false),
            )
            .unwrap();
        context
            .append_children(
                root.stable_id(),
                &[supported.stable_id(), opted_out.stable_id()],
            )
            .unwrap();
        context.assemble_color_field(supported).unwrap();
        context.assemble_color_field(opted_out).unwrap();

        // Host support starts false.
        let s_snap = context.read(supported, Clone::clone).unwrap();
        assert!(s_snap.eyedropper_button.is_none());
        let row_children = &context
            .world()
            .node(s_snap.bottom_row.unwrap())
            .unwrap()
            .children;
        assert_eq!(row_children, &[s_snap.hue_slider.unwrap()]);

        // Turning host support on reassembles existing fields without rebuilding opted-out ones.
        context.set_eyedropper_supported(true);
        let s_snap = context.read(supported, Clone::clone).unwrap();
        let o_snap = context.read(opted_out, Clone::clone).unwrap();
        let btn_id = s_snap.eyedropper_button.expect("eyedropper created");
        assert!(o_snap.eyedropper_button.is_none());
        assert_eq!(
            context
                .world()
                .node(s_snap.bottom_row.unwrap())
                .unwrap()
                .children,
            vec![btn_id, s_snap.hue_slider.unwrap()]
        );

        // Open the picker and verify bottom_row places [eyedropper | hue] cleanly side by side.
        assert!(context.activate_node(s_snap.picker.unwrap()).unwrap());
        context
            .layout_document(document, crate::LayoutViewport::new(640.0, 480.0))
            .unwrap();
        let btn_bounds = context.world().layout_box(btn_id).unwrap();
        let hue_bounds = context
            .world()
            .layout_box(s_snap.hue_slider.unwrap())
            .unwrap();
        let row_bounds = context
            .world()
            .layout_box(s_snap.bottom_row.unwrap())
            .unwrap();
        assert!(btn_bounds.width > 0.0 && hue_bounds.width > 0.0);
        assert!(
            hue_bounds.x >= btn_bounds.x + btn_bounds.width,
            "hue slider should start after eyedropper: btn={btn_bounds:?} hue={hue_bounds:?}"
        );
        assert!(
            hue_bounds.x + hue_bounds.width <= row_bounds.x + row_bounds.width + 0.5,
            "hue slider must fit inside bottom row: row={row_bounds:?} hue={hue_bounds:?}"
        );

        // Switching host support back off parks the eyedropper cleanly.
        context.set_eyedropper_supported(false);
        let s_snap = context.read(supported, Clone::clone).unwrap();
        assert_eq!(
            context
                .world()
                .node(s_snap.bottom_row.unwrap())
                .unwrap()
                .children,
            vec![s_snap.hue_slider.unwrap()]
        );
    }

    #[test]
    fn eyedropper_closes_popover_queues_request_and_completes_preserving_alpha() {
        let mut context = AppContext::new();
        context.set_eyedropper_supported(true);
        let document = DocumentId::new(1).unwrap();
        let field = context
            .create_component(document, ColorField::new([1.0, 0.0, 0.0, 0.35]))
            .unwrap();
        context.assemble_color_field(field).unwrap();
        let snap = context.read(field, Clone::clone).unwrap();
        let picker = Entity::<Popover>::from_stable_id(snap.picker.unwrap());
        let btn_id = snap.eyedropper_button.unwrap();

        let requested = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let req_hook = std::sync::Arc::clone(&requested);
        context
            .on(field, move |_, _: &ColorPickRequested, _| {
                req_hook.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            })
            .unwrap();
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let ev_in = std::sync::Arc::clone(&events);
        context
            .on(field, move |_, event: &ColorInput, _| {
                ev_in.lock().unwrap().push(("input", event.value));
            })
            .unwrap();
        let ev_ch = std::sync::Arc::clone(&events);
        context
            .on(field, move |_, event: &ColorChanged, _| {
                ev_ch.lock().unwrap().push(("changed", event.value));
            })
            .unwrap();

        assert!(context.activate_node(picker.stable_id()).unwrap());
        assert!(context.read(picker, |p| p.open).unwrap());

        assert!(context.activate_node(btn_id).unwrap());
        assert!(!context.read(picker, |p| p.open).unwrap());
        assert!(!context.read(field, |f| f.opened).unwrap());
        assert!(context.read(field, |f| f.sampling).unwrap());
        assert!(context.overlay_locks(document, field.stable_id()).menu_open);
        assert_eq!(requested.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(context.take_color_pick_requests(), vec![field.stable_id()]);
        assert!(context.take_color_pick_requests().is_empty());

        // Cancelling clears sampling without mutating value or emitting change events.
        assert!(context.complete_color_pick(field.stable_id(), None));
        assert!(!context.read(field, |f| f.sampling).unwrap());
        assert!(!context.overlay_locks(document, field.stable_id()).menu_open);
        assert!(events.lock().unwrap().is_empty());

        // Picking a color updates RGB, keeps alpha 0.35, syncs hex, and emits Input + Changed.
        assert!(context.activate_node(picker.stable_id()).unwrap());
        assert!(context.activate_node(btn_id).unwrap());
        let _ = context.take_color_pick_requests();
        assert!(context.complete_color_pick(field.stable_id(), Some([0.2, 0.4, 0.8])));
        assert!(!context.read(field, |f| f.sampling).unwrap());
        let updated = context.read(field, |f| f.value).unwrap();
        assert!((updated[0] - 0.2).abs() < 1e-5);
        assert!((updated[1] - 0.4).abs() < 1e-5);
        assert!((updated[2] - 0.8).abs() < 1e-5);
        assert!((updated[3] - 0.35).abs() < 1e-5);
        let hex_entity = Entity::<TextInput>::from_stable_id(snap.hex.unwrap());
        assert_eq!(
            context
                .read(hex_entity, |i| i.state.value.to_string())
                .unwrap(),
            "#3366cc59"
        );
        let evs = events.lock().unwrap();
        let kinds: Vec<_> = evs.iter().map(|(k, _)| *k).collect();
        assert_eq!(kinds, ["input", "changed"]);
    }
}
