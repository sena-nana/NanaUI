//! Input vocabulary shared by the canonical events and the hosts that make them.

/// The modifier keys held: on a live event, and in a stored shortcut chord.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct InputModifiers {
    pub alt: bool,
    pub control: bool,
    pub meta: bool,
    pub shift: bool,
}

impl InputModifiers {
    /// The platform's command modifier: Command on macOS, Control elsewhere.
    pub const fn primary() -> Self {
        if cfg!(target_os = "macos") {
            Self {
                meta: true,
                ..Self::empty()
            }
        } else {
            Self {
                control: true,
                ..Self::empty()
            }
        }
    }

    /// No modifier held.
    pub const fn empty() -> Self {
        Self {
            alt: false,
            control: false,
            meta: false,
            shift: false,
        }
    }

    pub const fn with_shift(mut self) -> Self {
        self.shift = true;
        self
    }

    pub const fn from_flags(control: bool, alt: bool, shift: bool, meta: bool) -> Self {
        Self {
            alt,
            control,
            meta,
            shift,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerPhase {
    Down,
    Move,
    Up,
    Cancel,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputDisposition {
    /// Whether Runtime/component handling consumed the event.
    pub handled: bool,
    /// Whether the host's default action must be suppressed.
    pub prevent_default: bool,
}
