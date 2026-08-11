use iced::Element;
use iced::widget::{container, tooltip};

use crate::theme::Colors;
use crate::widgets::tooltip_style;

pub use nana_ui_core::{TooltipConfig, TooltipPlacement};

fn iced_tooltip_position(placement: TooltipPlacement) -> iced::widget::tooltip::Position {
    match placement {
        TooltipPlacement::Top => iced::widget::tooltip::Position::Top,
        TooltipPlacement::Right => iced::widget::tooltip::Position::Right,
        TooltipPlacement::Bottom => iced::widget::tooltip::Position::Bottom,
        TooltipPlacement::Left => iced::widget::tooltip::Position::Left,
    }
}

pub(crate) fn tooltip_view<'a, Message: 'a>(
    trigger: impl Into<Element<'a, Message>>,
    content: impl Into<Element<'a, Message>>,
    config: TooltipConfig,
    colors: Colors,
) -> Element<'a, Message> {
    tooltip(
        trigger,
        container(content).width(config.max_width).padding([4, 7]),
        iced_tooltip_position(config.placement),
    )
    .gap(config.gap)
    .padding(config.viewport_padding)
    .delay(iced::time::Duration::from_millis(config.delay_ms))
    .snap_within_viewport(true)
    .style(tooltip_style(colors))
    .into()
}
