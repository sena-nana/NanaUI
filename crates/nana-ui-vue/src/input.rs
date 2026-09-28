//! Engine-neutral browser-style input contracts for Vue surfaces.

use std::collections::{BTreeMap, BTreeSet};

use nana_js_engine::HostValue;
pub use nana_ui_platform::{InputModifiers, PointerType};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HostedInputResult {
    pub targeted: bool,
    pub default_prevented: bool,
    /// Vue already performed the semantic action (for example `Press`).
    /// Hosts should skip a duplicate Scene-host path when this is set.
    pub consumed: bool,
}

fn extend_detail(modifiers: InputModifiers, detail: &mut BTreeMap<String, HostValue>) {
    detail.insert("altKey".into(), HostValue::Bool(modifiers.alt));
    detail.insert("ctrlKey".into(), HostValue::Bool(modifiers.control));
    detail.insert("metaKey".into(), HostValue::Bool(modifiers.meta));
    detail.insert("shiftKey".into(), HostValue::Bool(modifiers.shift));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerEventKind {
    Down,
    Move,
    Up,
    Cancel,
}

impl PointerEventKind {
    pub const fn pointer_name(self) -> &'static str {
        match self {
            Self::Down => "pointerdown",
            Self::Move => "pointermove",
            Self::Up => "pointerup",
            Self::Cancel => "pointercancel",
        }
    }

    pub const fn mouse_name(self) -> Option<&'static str> {
        match self {
            Self::Down => Some("mousedown"),
            Self::Move => Some("mousemove"),
            Self::Up => Some("mouseup"),
            Self::Cancel => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerInput {
    pub kind: PointerEventKind,
    pub pointer_id: u64,
    pub pointer_type: PointerType,
    pub is_primary: bool,
    pub client_x: f32,
    pub client_y: f32,
    pub screen_x: f32,
    pub screen_y: f32,
    pub button: i16,
    pub buttons: u16,
    pub pressure: f32,
    pub tangential_pressure: f32,
    pub tilt_x: i16,
    pub tilt_y: i16,
    pub twist: u16,
    pub modifiers: InputModifiers,
}

impl PointerInput {
    pub fn mouse(kind: PointerEventKind, x: f32, y: f32) -> Self {
        Self {
            kind,
            pointer_id: 1,
            pointer_type: PointerType::Mouse,
            is_primary: true,
            client_x: x,
            client_y: y,
            screen_x: x,
            screen_y: y,
            button: if matches!(kind, PointerEventKind::Move) {
                -1
            } else {
                0
            },
            buttons: if matches!(kind, PointerEventKind::Down) {
                1
            } else {
                0
            },
            pressure: if matches!(kind, PointerEventKind::Down) {
                0.5
            } else {
                0.0
            },
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            modifiers: InputModifiers::default(),
        }
    }

    pub(crate) fn detail(self) -> BTreeMap<String, HostValue> {
        let mut detail = BTreeMap::new();
        detail.insert(
            "pointerId".into(),
            HostValue::Number(self.pointer_id as f64),
        );
        detail.insert(
            "pointerType".into(),
            HostValue::string(self.pointer_type.as_str()),
        );
        detail.insert("isPrimary".into(), HostValue::Bool(self.is_primary));
        detail.insert("clientX".into(), HostValue::Number(self.client_x as f64));
        detail.insert("clientY".into(), HostValue::Number(self.client_y as f64));
        detail.insert("x".into(), HostValue::Number(self.client_x as f64));
        detail.insert("y".into(), HostValue::Number(self.client_y as f64));
        detail.insert("offsetX".into(), HostValue::Number(self.client_x as f64));
        detail.insert("offsetY".into(), HostValue::Number(self.client_y as f64));
        detail.insert("screenX".into(), HostValue::Number(self.screen_x as f64));
        detail.insert("screenY".into(), HostValue::Number(self.screen_y as f64));
        detail.insert("button".into(), HostValue::Number(self.button as f64));
        detail.insert("buttons".into(), HostValue::Number(self.buttons as f64));
        detail.insert("pressure".into(), HostValue::Number(self.pressure as f64));
        detail.insert(
            "tangentialPressure".into(),
            HostValue::Number(self.tangential_pressure as f64),
        );
        detail.insert("tiltX".into(), HostValue::Number(self.tilt_x as f64));
        detail.insert("tiltY".into(), HostValue::Number(self.tilt_y as f64));
        detail.insert("twist".into(), HostValue::Number(self.twist as f64));
        extend_detail(self.modifiers, &mut detail);
        detail
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WheelInput {
    pub client_x: f32,
    pub client_y: f32,
    pub screen_x: f32,
    pub screen_y: f32,
    pub delta_x: f32,
    pub delta_y: f32,
    /// DOM_DELTA_PIXEL = 0, DOM_DELTA_LINE = 1, DOM_DELTA_PAGE = 2.
    pub delta_mode: u8,
    pub modifiers: InputModifiers,
}

impl WheelInput {
    pub fn pixels(x: f32, y: f32, delta_x: f32, delta_y: f32) -> Self {
        Self {
            client_x: x,
            client_y: y,
            screen_x: x,
            screen_y: y,
            delta_x,
            delta_y,
            delta_mode: 0,
            modifiers: InputModifiers::default(),
        }
    }

    pub(crate) fn detail(self) -> BTreeMap<String, HostValue> {
        let mut detail = BTreeMap::new();
        detail.insert("clientX".into(), HostValue::Number(self.client_x as f64));
        detail.insert("clientY".into(), HostValue::Number(self.client_y as f64));
        detail.insert("screenX".into(), HostValue::Number(self.screen_x as f64));
        detail.insert("screenY".into(), HostValue::Number(self.screen_y as f64));
        detail.insert("deltaX".into(), HostValue::Number(self.delta_x as f64));
        detail.insert("deltaY".into(), HostValue::Number(self.delta_y as f64));
        detail.insert(
            "deltaMode".into(),
            HostValue::Number(self.delta_mode as f64),
        );
        extend_detail(self.modifiers, &mut detail);
        detail
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardEventKind {
    Down,
    Up,
}

impl KeyboardEventKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Down => "keydown",
            Self::Up => "keyup",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardInput {
    pub kind: KeyboardEventKind,
    pub key: String,
    pub code: String,
    pub location: u8,
    pub repeat: bool,
    pub composing: bool,
    pub modifiers: InputModifiers,
}

impl KeyboardInput {
    pub fn key_down(key: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            kind: KeyboardEventKind::Down,
            key: key.into(),
            code: code.into(),
            location: 0,
            repeat: false,
            composing: false,
            modifiers: InputModifiers::default(),
        }
    }

    pub(crate) fn detail(&self) -> BTreeMap<String, HostValue> {
        let mut detail = BTreeMap::new();
        detail.insert("key".into(), HostValue::string(&self.key));
        detail.insert("code".into(), HostValue::string(&self.code));
        detail.insert("location".into(), HostValue::Number(self.location as f64));
        detail.insert("repeat".into(), HostValue::Bool(self.repeat));
        detail.insert("isComposing".into(), HostValue::Bool(self.composing));
        extend_detail(self.modifiers, &mut detail);
        detail
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompositionEventKind {
    Start,
    Update,
    End,
}

impl CompositionEventKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Start => "compositionstart",
            Self::Update => "compositionupdate",
            Self::End => "compositionend",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositionInput {
    pub kind: CompositionEventKind,
    pub data: String,
}

impl CompositionInput {
    pub fn new(kind: CompositionEventKind, data: impl Into<String>) -> Self {
        Self {
            kind,
            data: data.into(),
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct InputState {
    pressed_keys: BTreeSet<String>,
}

impl InputState {
    pub fn note_key(&mut self, code: &str, pressed: bool) -> bool {
        if pressed {
            !self.pressed_keys.insert(code.to_string())
        } else {
            self.pressed_keys.remove(code);
            false
        }
    }

    pub fn clear(&mut self) {
        self.pressed_keys.clear();
    }
}

// The page's event shapes and the canonical input the Runtime routes carry
// the same fields; these convert at the boundary in both directions.
impl PointerInput {
    #[cfg(feature = "hosted")]
    pub(crate) fn from_canonical(pointer: &nana_ui_platform::PointerInput) -> Self {
        use nana_ui_platform::PointerPhase;
        Self {
            kind: match pointer.phase {
                PointerPhase::Down => PointerEventKind::Down,
                PointerPhase::Move => PointerEventKind::Move,
                PointerPhase::Up => PointerEventKind::Up,
                PointerPhase::Cancel => PointerEventKind::Cancel,
            },
            pointer_id: pointer.pointer_id.0,
            pointer_type: pointer.pointer_type,
            is_primary: pointer.is_primary,
            client_x: pointer.x,
            client_y: pointer.y,
            screen_x: pointer.screen_x,
            screen_y: pointer.screen_y,
            button: pointer.button,
            buttons: pointer.buttons,
            pressure: pointer.pressure,
            tangential_pressure: pointer.tangential_pressure,
            tilt_x: pointer.tilt_x,
            tilt_y: pointer.tilt_y,
            twist: pointer.twist,
            modifiers: pointer.modifiers,
        }
    }

    pub(crate) fn to_canonical(self) -> nana_ui_platform::PointerInput {
        use nana_ui_platform::PointerPhase;
        nana_ui_platform::PointerInput {
            phase: match self.kind {
                PointerEventKind::Down => PointerPhase::Down,
                PointerEventKind::Move => PointerPhase::Move,
                PointerEventKind::Up => PointerPhase::Up,
                PointerEventKind::Cancel => PointerPhase::Cancel,
            },
            pointer_id: nana_ui_platform::PointerId(self.pointer_id),
            pointer_type: self.pointer_type,
            x: self.client_x,
            y: self.client_y,
            screen_x: self.screen_x,
            screen_y: self.screen_y,
            button: self.button,
            buttons: self.buttons,
            pressure: self.pressure,
            tangential_pressure: self.tangential_pressure,
            tilt_x: self.tilt_x,
            tilt_y: self.tilt_y,
            twist: self.twist,
            is_primary: self.is_primary,
            activation_click: false,
            modifiers: self.modifiers,
        }
    }
}

impl WheelInput {
    #[cfg(feature = "hosted")]
    pub(crate) fn from_canonical(wheel: &nana_ui_platform::WheelInput) -> Self {
        Self {
            client_x: wheel.x,
            client_y: wheel.y,
            screen_x: wheel.x,
            screen_y: wheel.y,
            delta_x: wheel.delta_x,
            delta_y: wheel.delta_y,
            delta_mode: u8::from(wheel.unit == nana_ui_platform::WheelUnit::Lines),
            modifiers: wheel.modifiers,
        }
    }

    /// A page delta counts as lines: the Runtime knows pixels and lines.
    pub(crate) fn to_canonical(self) -> nana_ui_platform::WheelInput {
        nana_ui_platform::WheelInput {
            pointer_id: nana_ui_platform::PointerId(1),
            x: self.client_x,
            y: self.client_y,
            delta_x: self.delta_x,
            delta_y: self.delta_y,
            unit: if self.delta_mode == 0 {
                nana_ui_platform::WheelUnit::Pixels
            } else {
                nana_ui_platform::WheelUnit::Lines
            },
            modifiers: self.modifiers,
        }
    }
}

impl KeyboardInput {
    #[cfg(feature = "hosted")]
    pub(crate) fn from_canonical(key: &nana_ui_platform::KeyInput) -> Self {
        Self {
            kind: if key.is_pressed() {
                KeyboardEventKind::Down
            } else {
                KeyboardEventKind::Up
            },
            key: key.logical.0.to_string(),
            code: key.physical.0.to_string(),
            location: 0,
            repeat: key.repeat,
            composing: false,
            modifiers: key.modifiers,
        }
    }

    pub(crate) fn to_canonical(&self) -> nana_ui_platform::KeyInput {
        nana_ui_platform::KeyInput {
            physical: nana_ui_platform::PhysicalKey(self.code.clone().into()),
            logical: nana_ui_platform::LogicalKey(self.key.clone().into()),
            state: match self.kind {
                KeyboardEventKind::Down => nana_ui_platform::KeyState::Pressed,
                KeyboardEventKind::Up => nana_ui_platform::KeyState::Released,
            },
            repeat: self.repeat,
            modifiers: self.modifiers,
        }
    }
}

impl CompositionInput {
    /// What the platform would have sent for this page composition event:
    /// an end with text commits it.
    pub(crate) fn to_canonical(&self) -> nana_ui_platform::CompositionInput {
        match self.kind {
            CompositionEventKind::Start => nana_ui_platform::CompositionInput::Start,
            CompositionEventKind::Update => nana_ui_platform::CompositionInput::Update {
                text: self.data.clone(),
                selection: None,
            },
            CompositionEventKind::End if self.data.is_empty() => {
                nana_ui_platform::CompositionInput::End
            }
            CompositionEventKind::End => {
                nana_ui_platform::CompositionInput::Commit(self.data.clone())
            }
        }
    }
}
