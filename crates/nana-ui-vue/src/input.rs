//! Engine-neutral browser-style input contracts for Vue surfaces.

use std::collections::{BTreeMap, BTreeSet};

use nana_js_engine::HostValue;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HostedInputResult {
    pub targeted: bool,
    pub default_prevented: bool,
    /// Vue already performed the semantic action (for example `Press`).
    /// Hosts should skip the duplicate Iced path when this is set.
    pub consumed: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputModifiers {
    pub alt: bool,
    pub control: bool,
    pub meta: bool,
    pub shift: bool,
}

impl InputModifiers {
    pub(crate) fn extend_detail(self, detail: &mut BTreeMap<String, HostValue>) {
        detail.insert("altKey".into(), HostValue::Bool(self.alt));
        detail.insert("ctrlKey".into(), HostValue::Bool(self.control));
        detail.insert("metaKey".into(), HostValue::Bool(self.meta));
        detail.insert("shiftKey".into(), HostValue::Bool(self.shift));
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PointerType {
    #[default]
    Mouse,
    Touch,
    Pen,
}

impl PointerType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mouse => "mouse",
            Self::Touch => "touch",
            Self::Pen => "pen",
        }
    }
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
        self.modifiers.extend_detail(&mut detail);
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
        self.modifiers.extend_detail(&mut detail);
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
        self.modifiers.extend_detail(&mut detail);
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
