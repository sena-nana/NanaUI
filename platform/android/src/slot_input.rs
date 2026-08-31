//! NativeActivity pointer / key → NanaUI Runtime control-slot (host-testable).
//!
//! - Touch samples → platform [`InputEvent::Pointer`] in **logical** px.
//! - Key samples → platform [`InputEvent::Keyboard`]
//!   (US-QWERTY subset + named editing keys).
//!
//! Hit-testing uses the same viewport-bottom rect as [`crate::control_slot`].
//! The soft keyboard is driven by the activity loop's focus mirror (no
//! InputConnection, so no composition/preedit); accessibility publication
//! lives in [`crate::slot_ax`].

#![cfg_attr(not(target_os = "android"), allow(dead_code))]

use nana_ui_core::PhysicalRect;
use nana_ui_platform::{InputEvent, InputModifiers, PointerPhase, PointerType};

/// Touch / pointer phase from the host (Android MotionAction subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotTouchKind {
    Down,
    Move,
    Up,
    Cancel,
}

/// Modifier bits from the host (Android MetaState subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SlotKeyMods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub logo: bool,
}

impl SlotKeyMods {
    pub const fn to_input(self) -> InputModifiers {
        InputModifiers {
            alt: self.alt,
            control: self.ctrl,
            meta: self.logo,
            shift: self.shift,
        }
    }
}

/// Logical key for the control-slot (host-testable; no android-activity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotLogicalKey {
    Character(char),
    Backspace,
    Delete,
    Enter,
    Tab,
    Escape,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
}

impl SlotLogicalKey {
    pub fn as_input_key(self) -> String {
        match self {
            Self::Character(c) => c.to_string(),
            Self::Backspace => "Backspace".into(),
            Self::Delete => "Delete".into(),
            Self::Enter => "Enter".into(),
            Self::Tab => "Tab".into(),
            Self::Escape => "Escape".into(),
            Self::ArrowLeft => "ArrowLeft".into(),
            Self::ArrowRight => "ArrowRight".into(),
            Self::ArrowUp => "ArrowUp".into(),
            Self::ArrowDown => "ArrowDown".into(),
        }
    }

    pub fn committed_text(self) -> Option<String> {
        match self {
            Self::Character(c) => Some(c.to_string()),
            _ => None,
        }
    }
}

/// Whether a physical sample lies inside the control-slot scissor rect.
pub fn pointer_in_slot(slot: PhysicalRect, physical_x: f32, physical_y: f32) -> bool {
    if slot.width == 0 || slot.height == 0 {
        return false;
    }
    let x0 = slot.x as f32;
    let y0 = slot.y as f32;
    let x1 = x0 + slot.width as f32;
    let y1 = y0 + slot.height as f32;
    physical_x >= x0 && physical_x < x1 && physical_y >= y0 && physical_y < y1
}

/// Routes NativeActivity input to the NanaUI control-slot only.
///
/// Pointer events outside the slot stay `Unhandled` so VueHost can receive them.
/// Keyboard events are accepted only while the slot holds keyboard focus (last
/// Down was inside the slot); a Down outside clears that focus **when no
/// pointer gesture is captured**.
///
/// While a pointer is captured, additional Down samples (second finger / outside
/// tap) are ignored, and only that pointer id’s Move/Up/Cancel reach Runtime
/// (a secondary finger Up must not end the first press).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SlotInputGate {
    /// Android `MotionEvent` pointer id for the active in-slot gesture.
    pub captured_pointer_id: Option<i32>,
    /// Slot owns keyboard until the next Down outside the slot.
    pub keyboard_focused: bool,
}

impl SlotInputGate {
    /// Returns whether this sample should be delivered to Runtime (and marked Handled).
    pub fn accept_pointer(
        &mut self,
        slot: Option<PhysicalRect>,
        kind: SlotTouchKind,
        physical_x: f32,
        physical_y: f32,
        pointer_id: i32,
    ) -> bool {
        let in_slot = slot
            .map(|rect| pointer_in_slot(rect, physical_x, physical_y))
            .unwrap_or(false);
        match kind {
            SlotTouchKind::Down => {
                if self.captured_pointer_id.is_some() {
                    // Ignore secondary Down until the captured gesture ends.
                    return false;
                }
                self.captured_pointer_id = in_slot.then_some(pointer_id);
                // Down outside clears keyboard focus so keys are not swallowed.
                self.keyboard_focused = in_slot;
                in_slot
            }
            SlotTouchKind::Move => match self.captured_pointer_id {
                // Captured drag continues outside; only that pointer id.
                Some(id) => id == pointer_id,
                // Otherwise only hover inside (any pointer).
                None => in_slot,
            },
            SlotTouchKind::Up | SlotTouchKind::Cancel => {
                if self.captured_pointer_id != Some(pointer_id) {
                    // Secondary / unmatched Up must not release the capture.
                    return false;
                }
                self.captured_pointer_id = None;
                true
            }
        }
    }

    /// Keyboard samples reach Runtime only while the slot holds focus.
    pub fn accept_key(&self) -> bool {
        self.keyboard_focused
    }
}

/// Physical → logical for the Runtime viewport (`scale` = window scale factor).
pub fn logical_point(physical_x: f32, physical_y: f32, scale: f32) -> [f32; 2] {
    let scale = scale.max(0.25);
    [physical_x / scale, physical_y / scale]
}

/// Map one touch sample to a platform pointer event (logical window coords).
pub fn touch_to_pointer_event(
    kind: SlotTouchKind,
    logical: [f32; 2],
    pointer_id: i32,
    modifiers: InputModifiers,
) -> InputEvent {
    let (phase, button, buttons) = match kind {
        SlotTouchKind::Down => (PointerPhase::Down, 0, 1),
        SlotTouchKind::Move => (PointerPhase::Move, 0, 0),
        SlotTouchKind::Up => (PointerPhase::Up, 0, 0),
        SlotTouchKind::Cancel => (PointerPhase::Cancel, 0, 0),
    };
    InputEvent::Pointer {
        phase,
        pointer_id: pointer_id.max(0) as u64,
        pointer_type: PointerType::Touch,
        x: logical[0],
        y: logical[1],
        screen_x: logical[0],
        screen_y: logical[1],
        button,
        buttons,
        pressure: 1.0,
        tangential_pressure: 0.0,
        tilt_x: 0,
        tilt_y: 0,
        twist: 0,
        is_primary: true,
        activation_click: false,
        modifiers,
    }
}

/// Map one key sample to a platform keyboard event.
pub fn key_to_input_event(
    down: bool,
    key: SlotLogicalKey,
    modifiers: InputModifiers,
    repeat: bool,
) -> InputEvent {
    let name = key.as_input_key();
    InputEvent::Keyboard {
        pressed: down,
        text: if down { key.committed_text() } else { None },
        code: name.clone(),
        key: name,
        repeat,
        modifiers,
    }
}

/// Stable Android `AKEYCODE_*` integers (host-testable without NDK crates).
mod android_keycode {
    pub const DPAD_UP: u32 = 19;
    pub const DPAD_DOWN: u32 = 20;
    pub const DPAD_LEFT: u32 = 21;
    pub const DPAD_RIGHT: u32 = 22;
    pub const KEYCODE_0: u32 = 7;
    pub const KEYCODE_9: u32 = 16;
    pub const A: u32 = 29;
    pub const Z: u32 = 54;
    pub const COMMA: u32 = 55;
    pub const PERIOD: u32 = 56;
    pub const TAB: u32 = 61;
    pub const SPACE: u32 = 62;
    pub const ENTER: u32 = 66;
    pub const DEL: u32 = 67;
    pub const GRAVE: u32 = 68;
    pub const MINUS: u32 = 69;
    pub const EQUALS: u32 = 70;
    pub const LEFT_BRACKET: u32 = 71;
    pub const RIGHT_BRACKET: u32 = 72;
    pub const BACKSLASH: u32 = 73;
    pub const SEMICOLON: u32 = 74;
    pub const APOSTROPHE: u32 = 75;
    pub const SLASH: u32 = 76;
    pub const AT: u32 = 77;
    pub const PLUS: u32 = 81;
    pub const ESCAPE: u32 = 111;
    pub const FORWARD_DEL: u32 = 112;
    pub const SHIFT_LEFT: u32 = 59;
    pub const SHIFT_RIGHT: u32 = 60;
    pub const ALT_LEFT: u32 = 57;
    pub const ALT_RIGHT: u32 = 58;
    pub const CTRL_LEFT: u32 = 113;
    pub const CTRL_RIGHT: u32 = 114;
    pub const META_LEFT: u32 = 117;
    pub const META_RIGHT: u32 = 118;
    pub const CAPS_LOCK: u32 = 115;
}

/// True for pure modifier / lock keys (modifiers only; no character).
pub fn android_keycode_is_modifier(keycode: u32) -> bool {
    use android_keycode::*;
    matches!(
        keycode,
        SHIFT_LEFT
            | SHIFT_RIGHT
            | ALT_LEFT
            | ALT_RIGHT
            | CTRL_LEFT
            | CTRL_RIGHT
            | META_LEFT
            | META_RIGHT
            | CAPS_LOCK
    )
}

/// Map Android keycode + shift/caps to a slot logical key (US-QWERTY subset).
///
/// Returns `None` for system / media keys (Back, Home, Volume, …) so the host
/// can leave them `Unhandled`.
pub fn logical_key_from_android_keycode(
    keycode: u32,
    shift: bool,
    caps_lock: bool,
) -> Option<SlotLogicalKey> {
    use SlotLogicalKey::*;
    use android_keycode::*;

    match keycode {
        DEL => Some(Backspace),
        FORWARD_DEL => Some(Delete),
        ENTER => Some(Enter),
        TAB => Some(Tab),
        ESCAPE => Some(Escape),
        DPAD_LEFT => Some(ArrowLeft),
        DPAD_RIGHT => Some(ArrowRight),
        DPAD_UP => Some(ArrowUp),
        DPAD_DOWN => Some(ArrowDown),
        SPACE => Some(Character(' ')),
        AT => Some(Character('@')),
        PLUS => Some(Character('+')),
        A..=Z => {
            let base = b'a' + (keycode - A) as u8;
            let upper = shift ^ caps_lock;
            let c = if upper {
                (base as char).to_ascii_uppercase()
            } else {
                base as char
            };
            Some(Character(c))
        }
        KEYCODE_0..=KEYCODE_9 => {
            let digit = (b'0' + (keycode - KEYCODE_0) as u8) as char;
            if !shift {
                return Some(Character(digit));
            }
            let shifted = match digit {
                '1' => '!',
                '2' => '@',
                '3' => '#',
                '4' => '$',
                '5' => '%',
                '6' => '^',
                '7' => '&',
                '8' => '*',
                '9' => '(',
                '0' => ')',
                _ => digit,
            };
            Some(Character(shifted))
        }
        COMMA => Some(Character(if shift { '<' } else { ',' })),
        PERIOD => Some(Character(if shift { '>' } else { '.' })),
        SLASH => Some(Character(if shift { '?' } else { '/' })),
        SEMICOLON => Some(Character(if shift { ':' } else { ';' })),
        APOSTROPHE => Some(Character(if shift { '"' } else { '\'' })),
        LEFT_BRACKET => Some(Character(if shift { '{' } else { '[' })),
        RIGHT_BRACKET => Some(Character(if shift { '}' } else { ']' })),
        BACKSLASH => Some(Character(if shift { '|' } else { '\\' })),
        MINUS => Some(Character(if shift { '_' } else { '-' })),
        EQUALS => Some(Character(if shift { '+' } else { '=' })),
        GRAVE => Some(Character(if shift { '~' } else { '`' })),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_in_slot_bounds() {
        let slot = PhysicalRect {
            x: 10,
            y: 100,
            width: 200,
            height: 48,
        };
        assert!(pointer_in_slot(slot, 10.0, 100.0));
        assert!(pointer_in_slot(slot, 209.9, 147.9));
        assert!(!pointer_in_slot(slot, 9.9, 100.0));
        assert!(!pointer_in_slot(slot, 10.0, 148.0));
        assert!(!pointer_in_slot(slot, 210.0, 100.0));
    }

    fn test_slot() -> PhysicalRect {
        PhysicalRect {
            x: 10,
            y: 100,
            width: 200,
            height: 48,
        }
    }

    #[test]
    fn gate_rejects_pointer_outside_slot() {
        let mut gate = SlotInputGate::default();
        let slot = Some(test_slot());
        assert!(!gate.accept_pointer(slot, SlotTouchKind::Down, 5.0, 50.0, 0));
        assert!(gate.captured_pointer_id.is_none());
        assert!(!gate.keyboard_focused);
        assert!(!gate.accept_key());
        assert!(!gate.accept_pointer(slot, SlotTouchKind::Move, 5.0, 50.0, 0));
        assert!(!gate.accept_pointer(slot, SlotTouchKind::Up, 5.0, 50.0, 0));
    }

    #[test]
    fn gate_accepts_pointer_inside_and_captures_drag() {
        let mut gate = SlotInputGate::default();
        let slot = Some(test_slot());
        assert!(gate.accept_pointer(slot, SlotTouchKind::Down, 20.0, 120.0, 0));
        assert!(gate.captured_pointer_id.is_some());
        assert_eq!(gate.captured_pointer_id, Some(0));
        assert!(gate.keyboard_focused);
        assert!(gate.accept_key());
        assert!(gate.accept_pointer(slot, SlotTouchKind::Move, 1.0, 1.0, 0));
        assert!(gate.accept_pointer(slot, SlotTouchKind::Up, 1.0, 1.0, 0));
        assert!(gate.captured_pointer_id.is_none());
        assert!(gate.accept_key());
    }

    #[test]
    fn gate_down_outside_clears_keyboard_focus() {
        let mut gate = SlotInputGate::default();
        let slot = Some(test_slot());
        assert!(gate.accept_pointer(slot, SlotTouchKind::Down, 20.0, 120.0, 0));
        assert!(gate.accept_pointer(slot, SlotTouchKind::Up, 20.0, 120.0, 0));
        assert!(gate.accept_key());
        assert!(!gate.accept_pointer(slot, SlotTouchKind::Down, 1.0, 1.0, 0));
        assert!(!gate.keyboard_focused);
        assert!(!gate.accept_key());
    }

    #[test]
    fn gate_ignores_second_down_while_captured() {
        let mut gate = SlotInputGate::default();
        let slot = Some(test_slot());
        assert!(gate.accept_pointer(slot, SlotTouchKind::Down, 20.0, 120.0, 0));
        assert!(gate.captured_pointer_id.is_some());
        assert!(gate.keyboard_focused);
        assert!(!gate.accept_pointer(slot, SlotTouchKind::Down, 1.0, 1.0, 1));
        assert!(gate.captured_pointer_id.is_some());
        assert_eq!(gate.captured_pointer_id, Some(0));
        assert!(gate.keyboard_focused);
        assert!(gate.accept_pointer(slot, SlotTouchKind::Move, 1.0, 1.0, 0));
        assert!(gate.accept_pointer(slot, SlotTouchKind::Up, 1.0, 1.0, 0));
        assert!(gate.captured_pointer_id.is_none());
        assert!(gate.accept_key());
    }

    #[test]
    fn gate_ignores_secondary_pointer_up_while_captured() {
        let mut gate = SlotInputGate::default();
        let slot = Some(test_slot());
        assert!(gate.accept_pointer(slot, SlotTouchKind::Down, 20.0, 120.0, 7));
        assert_eq!(gate.captured_pointer_id, Some(7));
        assert!(!gate.accept_pointer(slot, SlotTouchKind::Up, 50.0, 50.0, 8));
        assert!(!gate.accept_pointer(slot, SlotTouchKind::Cancel, 50.0, 50.0, 8));
        assert!(gate.captured_pointer_id.is_some());
        assert_eq!(gate.captured_pointer_id, Some(7));
        assert!(!gate.accept_pointer(slot, SlotTouchKind::Move, 1.0, 1.0, 8));
        assert!(gate.accept_pointer(slot, SlotTouchKind::Move, 1.0, 1.0, 7));
        assert!(gate.accept_pointer(slot, SlotTouchKind::Up, 1.0, 1.0, 7));
        assert!(gate.captured_pointer_id.is_none());
    }

    #[test]
    fn gate_rejects_all_when_slot_missing() {
        let mut gate = SlotInputGate::default();
        assert!(!gate.accept_pointer(None, SlotTouchKind::Down, 20.0, 120.0, 0));
        assert!(!gate.accept_key());
    }

    #[test]
    fn gate_hover_inside_without_capture() {
        let mut gate = SlotInputGate::default();
        let slot = Some(test_slot());
        assert!(gate.accept_pointer(slot, SlotTouchKind::Move, 20.0, 120.0, 0));
        assert!(gate.captured_pointer_id.is_none());
        assert!(!gate.keyboard_focused);
    }

    #[test]
    fn logical_point_divides_by_scale() {
        let p = logical_point(216.0, 432.0, 2.0);
        assert!((p[0] - 108.0).abs() < 0.01);
        assert!((p[1] - 216.0).abs() < 0.01);
    }

    #[test]
    fn down_emits_primary_pointer_down() {
        let event = touch_to_pointer_event(
            SlotTouchKind::Down,
            [1.0, 2.0],
            3,
            InputModifiers::default(),
        );
        match event {
            InputEvent::Pointer {
                phase,
                pointer_id,
                pointer_type,
                x,
                y,
                is_primary,
                button,
                ..
            } => {
                assert_eq!(phase, PointerPhase::Down);
                assert_eq!(pointer_id, 3);
                assert_eq!(pointer_type, PointerType::Touch);
                assert!((x - 1.0).abs() < f32::EPSILON);
                assert!((y - 2.0).abs() < f32::EPSILON);
                assert!(is_primary);
                assert_eq!(button, 0);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn letter_keycode_respects_shift_and_caps() {
        assert_eq!(
            logical_key_from_android_keycode(29, false, false),
            Some(SlotLogicalKey::Character('a'))
        );
        assert_eq!(
            logical_key_from_android_keycode(29, true, false),
            Some(SlotLogicalKey::Character('A'))
        );
        assert_eq!(
            logical_key_from_android_keycode(29, false, true),
            Some(SlotLogicalKey::Character('A'))
        );
        assert_eq!(
            logical_key_from_android_keycode(29, true, true),
            Some(SlotLogicalKey::Character('a'))
        );
    }

    #[test]
    fn digit_and_backspace_map() {
        assert_eq!(
            logical_key_from_android_keycode(8, false, false),
            Some(SlotLogicalKey::Character('1'))
        );
        assert_eq!(
            logical_key_from_android_keycode(8, true, false),
            Some(SlotLogicalKey::Character('!'))
        );
        assert_eq!(
            logical_key_from_android_keycode(67, false, false),
            Some(SlotLogicalKey::Backspace)
        );
    }

    #[test]
    fn system_back_unmapped() {
        assert_eq!(logical_key_from_android_keycode(4, false, false), None);
    }

    #[test]
    fn key_press_emits_text_for_character() {
        let event = key_to_input_event(
            true,
            SlotLogicalKey::Character('x'),
            InputModifiers::default(),
            false,
        );
        match event {
            InputEvent::Keyboard {
                pressed,
                key,
                text: Some(t),
                ..
            } => {
                assert!(pressed);
                assert_eq!(key, "x");
                assert_eq!(t, "x");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn key_press_backspace_is_named() {
        let event = key_to_input_event(
            true,
            SlotLogicalKey::Backspace,
            InputModifiers::default(),
            false,
        );
        match event {
            InputEvent::Keyboard {
                key,
                text: None,
                pressed: true,
                ..
            } => assert_eq!(key, "Backspace"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn mods_to_input_sets_bits() {
        let mods = SlotKeyMods {
            shift: true,
            ctrl: true,
            alt: false,
            logo: false,
        }
        .to_input();
        assert!(mods.shift);
        assert!(mods.control);
        assert!(!mods.alt);
        assert!(!mods.meta);
    }

    #[test]
    fn modifier_keycodes_detected() {
        assert!(android_keycode_is_modifier(59)); // SHIFT_LEFT
        assert!(android_keycode_is_modifier(113)); // CTRL_LEFT
        assert!(!android_keycode_is_modifier(29)); // A
    }
}
