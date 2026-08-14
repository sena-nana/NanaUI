//! IME / soft-keyboard host boundary.
//!
//! Desktop hosted windows use winit's IME enable, cursor area, preedit, and
//! commit path.

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
