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

    /// Square extent of a checkbox box or radio ring.
    pub const fn indicator_size(self) -> f32 {
        match self {
            Self::Small => 14.0,
            Self::Medium => 16.0,
            Self::Large => 18.0,
        }
    }

    /// Gap between an indicator and its label.
    pub const fn indicator_gap(self) -> f32 {
        match self {
            Self::Small => 6.0,
            Self::Medium => 8.0,
            Self::Large => 8.0,
        }
    }

    /// Left inset a radio row reserves before its label: a small row inset,
    /// the ring, and the label gap.
    pub const fn radio_lead(self) -> f32 {
        RADIO_ROW_INSET + self.indicator_size() + self.indicator_gap()
    }
}

/// Left inset of a radio ring inside its row, so hover chrome is not flush.
pub const RADIO_ROW_INSET: f32 = 4.0;

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

/// Placement options for hover tooltips.
///
/// The default follows the pointer. Directional variants stay anchored to the
/// trigger and flip only when the preferred side cannot fit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TooltipPlacement {
    #[default]
    FollowCursor,
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

impl TooltipConfig {
    pub const PADDING_X: f32 = 7.0;
    pub const PADDING_Y: f32 = 4.0;
    pub const RADIUS: f32 = 4.0;
    pub const FONT_SIZE: f32 = 11.0;
}

impl Default for TooltipConfig {
    fn default() -> Self {
        Self {
            placement: TooltipPlacement::FollowCursor,
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
pub enum PopoverAlignment {
    Start,
    #[default]
    Center,
    End,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DrawerSide {
    Left,
    #[default]
    Right,
    Bottom,
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

    /// True when `(x, y)` sits in the platform traffic-light / caption exclusion.
    ///
    /// Title-bar drag and app chrome hit-testing should ignore this band so
    /// native controls keep the event. Insets of `0` never exclude.
    pub fn native_control_hit(self, bar: crate::LogicalRect, x: f32, y: f32) -> bool {
        native_control_hit(
            bar,
            valid_inset(self.leading_inset),
            valid_inset(self.trailing_inset),
            x,
            y,
        )
    }
}

pub(crate) fn native_control_hit(
    bar: crate::LogicalRect,
    leading_inset: f32,
    trailing_inset: f32,
    x: f32,
    y: f32,
) -> bool {
    if x < bar.x || y < bar.y || x >= bar.x + bar.width || y >= bar.y + bar.height {
        return false;
    }
    let local_x = x - bar.x;
    (leading_inset > 0.0 && local_x < leading_inset)
        || (trailing_inset > 0.0 && local_x >= (bar.width - trailing_inset).max(0.0))
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
