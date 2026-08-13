//! IME / soft-keyboard host boundary.
//!
//! Android NativeActivity MVP keeps [`UnsupportedIme`]: showing the soft keyboard
//! without an `InputConnection` (GameActivity / custom Java) does not deliver
//! text on modern IMEs. Desktop hosted windows use winit's IME enable, cursor
//! area, preedit, and commit path.

/// Engine-neutral text-input purpose used when selecting the OS keyboard mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ImePurpose {
    #[default]
    Normal,
    Password,
    Terminal,
}

/// Candidate-window anchor in logical window coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImeCursorArea {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// High-level IME visibility request from UI → platform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImeRequest {
    Hide,
    Show,
    SetCursorArea(ImeCursorArea),
    SetPurpose(ImePurpose),
}

/// Native IME lifecycle delivered to Vue/custom editors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImeEvent {
    Enabled,
    Disabled,
    Preedit {
        text: String,
        selection: Option<(usize, usize)>,
    },
    Commit(String),
}

/// Host that can show/hide the platform soft keyboard.
pub trait ImeHost {
    fn request(&mut self, request: ImeRequest);
}

/// No-op IME host (Android NativeActivity MVP / desktop stub).
#[derive(Debug, Default, Clone, Copy)]
pub struct UnsupportedIme;

impl ImeHost for UnsupportedIme {
    fn request(&mut self, _request: ImeRequest) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_ime_accepts_requests() {
        let mut ime = UnsupportedIme;
        ime.request(ImeRequest::Show);
        ime.request(ImeRequest::Hide);
        ime.request(ImeRequest::SetCursorArea(ImeCursorArea {
            x: 12.0,
            y: 24.0,
            width: 2.0,
            height: 20.0,
        }));
        ime.request(ImeRequest::SetPurpose(ImePurpose::Normal));
    }
}
