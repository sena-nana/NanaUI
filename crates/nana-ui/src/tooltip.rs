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

impl From<TooltipPlacement> for iced::widget::tooltip::Position {
    fn from(placement: TooltipPlacement) -> Self {
        match placement {
            TooltipPlacement::Top => Self::Top,
            TooltipPlacement::Right => Self::Right,
            TooltipPlacement::Bottom => Self::Bottom,
            TooltipPlacement::Left => Self::Left,
        }
    }
}
