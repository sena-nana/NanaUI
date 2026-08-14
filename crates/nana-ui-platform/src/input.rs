//! Renderer-neutral host input delivered to Nana runtimes and adapters.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputModifiers {
    pub alt: bool,
    pub control: bool,
    pub meta: bool,
    pub shift: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    Pointer {
        phase: PointerPhase,
        pointer_id: u64,
        pointer_type: PointerType,
        x: f32,
        y: f32,
        screen_x: f32,
        screen_y: f32,
        button: i16,
        buttons: u16,
        pressure: f32,
        tangential_pressure: f32,
        tilt_x: i16,
        tilt_y: i16,
        twist: u16,
        is_primary: bool,
        modifiers: InputModifiers,
    },
    Wheel {
        x: f32,
        y: f32,
        delta_x: f32,
        delta_y: f32,
        line_delta: bool,
        modifiers: InputModifiers,
    },
    Keyboard {
        pressed: bool,
        key: String,
        /// Committed text supplied by the platform key event. `None` while an
        /// IME owns composition; adapters must not derive text from `key`.
        text: Option<String>,
        code: String,
        repeat: bool,
        modifiers: InputModifiers,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerPhase {
    Down,
    Move,
    Up,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerType {
    Mouse,
    Touch,
    Pen,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputDisposition {
    pub prevent_default: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_contract_preserves_pen_and_modifier_data() {
        let event = InputEvent::Pointer {
            phase: PointerPhase::Move,
            pointer_id: 7,
            pointer_type: PointerType::Pen,
            x: 12.0,
            y: 24.0,
            screen_x: 120.0,
            screen_y: 240.0,
            button: 0,
            buttons: 1,
            pressure: 0.75,
            tangential_pressure: -0.25,
            tilt_x: 15,
            tilt_y: -10,
            twist: 180,
            is_primary: true,
            modifiers: InputModifiers {
                shift: true,
                ..InputModifiers::default()
            },
        };
        let InputEvent::Pointer {
            pointer_type,
            pressure,
            modifiers,
            ..
        } = event
        else {
            panic!("expected pointer event");
        };
        assert_eq!(pointer_type, PointerType::Pen);
        assert_eq!(pressure, 0.75);
        assert!(modifiers.shift);
    }
}
