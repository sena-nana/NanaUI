//! Color well plus hex field. The picker surface is assembled children
//! (`XYPad` saturation/value, `RangeField` hue); the host never takes a
//! window handle.

use std::sync::Arc;

use nana_ui_core::{
    AlignSpec, ControlSize, FlexDirection, JustifySpec, LengthSpec, SemanticColorRole, UI_METRICS,
};

use crate::view_components::{
    Button, RangeChanged, RangeField, TextChanged, TextInput, project_common,
};
use crate::{
    AccessibilityRole, AccessibilityState, Activate, AppContext, ComponentView, Entity,
    FrameworkError, InteractionState, MutationQueue, NodeKind, NodeStyle, Popover, StableNodeId,
    TextContent, UiWorld, XYPad, XYPadEvent, XYPadValue,
};

const SWATCH_SIZE: f32 = 22.0;

/// Committed RGBA in 0..=1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorChanged {
    pub value: [f32; 4],
}

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
    pub opened: bool,
    pub disabled: bool,
    pub invalid: bool,
    pub size: ControlSize,
    pub swatch: Option<StableNodeId>,
    pub hex: Option<StableNodeId>,
    pub picker: Option<StableNodeId>,
    pub pad: Option<StableNodeId>,
    pub hue_slider: Option<StableNodeId>,
    pub style: NodeStyle,
}

impl ColorField {
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
            swatch: None,
            hex: None,
            picker: None,
            pad: None,
            hue_slider: None,
            style: field_style(ControlSize::Medium),
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
        if self.invalid {
            style.border = Some(SemanticColorRole::Danger);
        }
        let layout = std::sync::Arc::make_mut(&mut style.layout);
        layout.direction = Some(FlexDirection::Row);
        layout.align_items = AlignSpec::Center;
        layout.justify_content = JustifySpec::Start;
        layout.gap = Some(LengthSpec::Px(6.0));
        layout.width = Some(LengthSpec::Fill);
        layout.min_height = Some(LengthSpec::Px(self.size.height()));
        style
    }
}

impl ComponentView for ColorField {
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
                    value: String::new(),
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
                label: Some(Arc::from("颜色")),
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
        let swatch = match snapshot.swatch.filter(|id| self.world().contains(*id)) {
            Some(id) => Entity::<Button>::from_stable_id(id),
            None => self.create_detached_component(document, swatch_button(snapshot.value))?,
        };
        let hex = match snapshot.hex.filter(|id| self.world().contains(*id)) {
            Some(id) => Entity::<TextInput>::from_stable_id(id),
            None => self.create_detached_component(
                document,
                TextInput::new(format_hex(snapshot.value)).size(snapshot.size),
            )?,
        };
        let pad = match snapshot.pad.filter(|id| self.world().contains(*id)) {
            Some(id) => Entity::<XYPad>::from_stable_id(id),
            None => self.create_detached_component(
                document,
                XYPad::new(XYPadValue {
                    x: snapshot.sat,
                    y: snapshot.val,
                })
                .x_range(0.0, 1.0)
                .y_range(0.0, 1.0),
            )?,
        };
        let hue = match snapshot.hue_slider.filter(|id| self.world().contains(*id)) {
            Some(id) => Entity::<RangeField>::from_stable_id(id),
            None => self.create_detached_component(
                document,
                RangeField::new(snapshot.hue as f64, 0.0, 360.0, 1.0)
                    .expect("hue range"),
            )?,
        };
        let picker = match snapshot.picker.filter(|id| self.world().contains(*id)) {
            Some(id) => Entity::<Popover>::from_stable_id(id),
            None => self.create_detached_component(
                document,
                Popover::new().width(220.0).open(snapshot.opened),
            )?,
        };

        if created {
            self.observe(swatch, field, |field, _: &Activate, _cx| {
                if !field.disabled {
                    field.opened = !field.opened;
                }
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
                match event {
                    XYPadEvent::Input(_) => cx.emit(ColorInput { value: field.value }),
                    XYPadEvent::Change(_) => cx.emit(ColorChanged { value: field.value }),
                }
            })?;
            self.observe(hue, field, |field, event: &RangeChanged, cx| {
                field.hue = event.value as f32;
                field.apply_hsv();
                cx.emit(ColorInput { value: field.value });
            })?;
        }

        self.update_component(swatch, |button, _| {
            *button = swatch_button(snapshot.value);
            button.disabled = snapshot.disabled;
        })?;
        self.update_component(hex, |input, _| {
            let next = format_hex(snapshot.value);
            if input.state.value != next {
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
        })?;
        self.update_component(hue, |range, _| {
            range.value = snapshot.hue as f64;
            range.disabled = snapshot.disabled;
        })?;
        self.update_component(picker, |popover, _| {
            popover.open = snapshot.opened && !snapshot.disabled;
        })?;
        self.update_component(field, |field, _| {
            field.swatch = Some(swatch.stable_id());
            field.hex = Some(hex.stable_id());
            field.picker = Some(picker.stable_id());
            field.pad = Some(pad.stable_id());
            field.hue_slider = Some(hue.stable_id());
        })?;

        self.append_child(picker, pad)?;
        self.append_child(picker, hue)?;
        self.append_child(field, swatch)?;
        self.append_child(field, hex)?;
        self.append_child(field, picker)?;
        Ok(created)
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

fn field_style(size: ControlSize) -> NodeStyle {
    let mut style = NodeStyle::default();
    style.background = Some(SemanticColorRole::Subtle);
    style.border = Some(SemanticColorRole::Border);
    let layout = std::sync::Arc::make_mut(&mut style.layout);
    layout.padding_left = Some(LengthSpec::Px(6.0));
    layout.padding_right = Some(LengthSpec::Px(6.0));
    layout.border_width = Some(1.0);
    layout.border_radius = Some(UI_METRICS.radius_sm);
    layout.min_height = Some(LengthSpec::Px(size.height()));
    style
}

fn swatch_button(value: [f32; 4]) -> Button {
    let mut button = Button::new("");
    let layout = std::sync::Arc::make_mut(&mut button.style.layout);
    layout.width = Some(LengthSpec::Px(SWATCH_SIZE));
    layout.height = Some(LengthSpec::Px(SWATCH_SIZE));
    layout.min_width = Some(LengthSpec::Px(SWATCH_SIZE));
    layout.min_height = Some(LengthSpec::Px(SWATCH_SIZE));
    layout.flex_grow = Some(0.0);
    layout.flex_shrink = Some(0.0);
    layout.border_radius = Some(UI_METRICS.radius_sm);
    layout.border_width = Some(1.0);
    layout.background = Some(sanitize_rgba(value));
    button.style.border = Some(SemanticColorRole::Border);
    button
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
    let sat = if max <= f32::EPSILON { 0.0 } else { delta / max };
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
}
