//! IME / soft-keyboard host boundary.
//!
//! Android NativeActivity MVP keeps [`UnsupportedIme`]: showing the soft keyboard
//! without an `InputConnection` (GameActivity / custom Java) does not deliver
//! text on modern IMEs. Do not flip `PlatformCapabilities::ime` until commit
//! events reach iced.

/// High-level IME visibility request from UI → platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImeRequest {
    Hide,
    Show,
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
    }
}
