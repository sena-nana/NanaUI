//! Control / overlay / chrome **semantics** — Style Model Semantics slice.
//!
//! Shared by L2 Vue props and L3 Rust builders. L1 maps known classes/roles here
//! via `nana-ui-vue::widget_map`; it does not invent ThemeTokens from paint CSS.

use crate::theme::{ThemeMetrics, UI_BASE_TEXT_SIZE, UI_METRICS};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ControlSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl ControlSize {
    pub const fn height(self) -> f32 {
        self.height_in(UI_METRICS)
    }

    pub const fn height_in(self, metrics: ThemeMetrics) -> f32 {
        match self {
            Self::Small => metrics.small_control_height(),
            Self::Medium => metrics.medium_control_height(),
            Self::Large => metrics.large_control_height(),
        }
    }

    pub const fn line_height(self) -> f32 {
        match self {
            Self::Small | Self::Medium => 16.0,
            Self::Large => 18.0,
        }
    }

    pub const fn vertical_padding(self, metrics: ThemeMetrics) -> f32 {
        let remaining = self.height_in(metrics) - self.line_height();
        if remaining > 0.0 {
            remaining / 2.0
        } else {
            0.0
        }
    }

    pub fn nearest(height: f32) -> Self {
        if !height.is_finite() {
            return Self::Medium;
        }
        if height <= (Self::Small.height() + Self::Medium.height()) / 2.0 {
            Self::Small
        } else if height <= (Self::Medium.height() + Self::Large.height()) / 2.0 {
            Self::Medium
        } else {
            Self::Large
        }
    }

    pub const fn padding_x(self) -> f32 {
        match self {
            Self::Small => 8.0,
            Self::Medium => UI_METRICS.control_padding_x,
            Self::Large => 14.0,
        }
    }

    pub const fn text_size(self) -> f32 {
        match self {
            Self::Small => UI_BASE_TEXT_SIZE - 1.0,
            Self::Medium => UI_BASE_TEXT_SIZE,
            Self::Large => UI_BASE_TEXT_SIZE + 1.0,
        }
    }

    pub const fn icon_size(self) -> f32 {
        match self {
            Self::Small => 13.0,
            Self::Medium => 14.0,
            Self::Large => 16.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Ghost,
    Subtle,
    Selected,
    Primary,
    Warning,
    Danger,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardKind {
    Surface,
    Outlined,
    Raised,
    Flat,
    Selected,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SwitchControlPosition {
    Start,
    #[default]
    End,
}

/// Placement options shared with LiliaUI tooltips.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TooltipPlacement {
    #[default]
    Top,
    Right,
    Bottom,
    Left,
}

/// Visual and timing defaults for a tooltip.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TooltipConfig {
    pub placement: TooltipPlacement,
    pub delay_ms: u64,
    pub gap: f32,
    pub viewport_padding: f32,
    pub max_width: f32,
}

impl Default for TooltipConfig {
    fn default() -> Self {
        Self {
            placement: TooltipPlacement::Top,
            delay_ms: 350,
            gap: 6.0,
            viewport_padding: 4.0,
            max_width: 280.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PopoverPlacement {
    Top,
    #[default]
    Bottom,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DrawerSide {
    Left,
    #[default]
    Right,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AnchoredMenuPlacement {
    TopStart,
    TopEnd,
    #[default]
    BottomStart,
    BottomEnd,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StatusTone {
    #[default]
    Neutral,
    Info,
    Success,
    Warning,
    Danger,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ToastTone {
    #[default]
    Info,
    Success,
    Warning,
    Danger,
}

impl ToastTone {
    pub fn status(self) -> StatusTone {
        match self {
            Self::Info => StatusTone::Info,
            Self::Success => StatusTone::Success,
            Self::Warning => StatusTone::Warning,
            Self::Danger => StatusTone::Danger,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationIntent {
    Warning,
    Danger,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropdownSelection<T> {
    Single(Option<T>),
    Multiple(Vec<T>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropdownEvent<T> {
    Select(T),
    Toggle(T),
    Opened,
    Closed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct XYPadValue {
    pub x: f32,
    pub y: f32,
}

impl XYPadValue {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum XYPadEvent {
    Input(XYPadValue),
    Change(XYPadValue),
}

/// Selects whether window controls are supplied by the platform or NanaUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowControlMode {
    NativeLeading,
    NativeTrailing,
    Custom,
}

/// Platform presentation contract for a NanaUI application title bar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowChrome {
    pub controls: WindowControlMode,
    pub leading_inset: f32,
    pub trailing_inset: f32,
}

impl WindowChrome {
    pub fn new(controls: WindowControlMode, leading_inset: f32, trailing_inset: f32) -> Self {
        Self {
            controls,
            leading_inset: valid_inset(leading_inset),
            trailing_inset: valid_inset(trailing_inset),
        }
    }

    pub const fn custom() -> Self {
        Self {
            controls: WindowControlMode::Custom,
            leading_inset: 0.0,
            trailing_inset: 0.0,
        }
    }

    pub fn native_leading(leading_inset: f32) -> Self {
        Self {
            controls: WindowControlMode::NativeLeading,
            leading_inset: valid_inset(leading_inset),
            trailing_inset: 0.0,
        }
    }

    pub fn native_trailing(trailing_inset: f32) -> Self {
        Self {
            controls: WindowControlMode::NativeTrailing,
            leading_inset: 0.0,
            trailing_inset: valid_inset(trailing_inset),
        }
    }

    pub const fn uses_custom_controls(self) -> bool {
        matches!(self.controls, WindowControlMode::Custom)
    }

    pub fn platform_default() -> Self {
        #[cfg(target_os = "macos")]
        {
            const MACOS_TRAFFIC_LIGHT_INSET: f32 = 78.0;
            Self::native_leading(MACOS_TRAFFIC_LIGHT_INSET)
        }

        #[cfg(not(target_os = "macos"))]
        {
            Self::custom()
        }
    }
}

impl Default for WindowChrome {
    fn default() -> Self {
        Self::platform_default()
    }
}

/// A real operation requested by the custom title bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowChromeAction {
    Drag,
    Minimize,
    ToggleMaximize,
    Close,
}

fn valid_inset(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

pub use crate::settings::AppearanceEvent;
