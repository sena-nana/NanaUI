//! Input vocabulary shared by the canonical events and the hosts that make them.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputModifiers {
    pub alt: bool,
    pub control: bool,
    pub meta: bool,
    pub shift: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerPhase {
    Down,
    Move,
    Up,
    Cancel,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputDisposition {
    /// Whether Runtime/component handling consumed the event.
    pub handled: bool,
    /// Whether the host's default action must be suppressed.
    pub prevent_default: bool,
}
