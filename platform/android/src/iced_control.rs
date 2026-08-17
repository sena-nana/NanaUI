//! Experimental Iced control-slot widgets (not the product Runtime path).
//!
//! Android is not a NanaUI product target. This strip uses raw Iced widgets
//! plus Nana `Icon` identity; it does not depend on removed compatibility
//! adapters.

use std::borrow::Cow;

use iced::widget::{button, row, text, toggler};
use iced::{Alignment, Color, Element, Length};
use nana_ui::{Icon, ThemeTokens};

pub const SLOT_BUTTON_LABEL: &str = "Nana";
pub const SLOT_TEXT_LABEL: &str = "Shell";
pub const SLOT_ICON: Icon = Icon::Settings;
pub const SLOT_ICON_SIZE: f32 = 18.0;
pub const SLOT_SWITCH_LABEL: &str = "On";
pub const SLOT_INPUT_PLACEHOLDER: &str = "Type…";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotStripMessage {
    Pressed,
    Toggle(bool),
    Input(String),
}

pub fn slot_strip_element<'a>(
    switch_on: bool,
    input_value: impl Into<Cow<'a, str>>,
    button_label: impl Into<Cow<'a, str>>,
    tokens: ThemeTokens,
) -> Element<'a, SlotStripMessage> {
    let leading = text("⚙")
        .size(SLOT_ICON_SIZE)
        .color(iced_color(tokens.colors.muted));
    let caption = text(SLOT_TEXT_LABEL)
        .size(14)
        .color(iced_color(tokens.colors.text));
    let input_value = input_value.into().into_owned();
    let field = text(input_value).size(13);
    let switch = toggler(switch_on).on_toggle(SlotStripMessage::Toggle);
    let action = button(text(button_label.into()).size(13)).on_press(SlotStripMessage::Pressed);
    row![leading, caption, field, switch, action]
        .spacing(10)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .into()
}

fn iced_color(color: nana_ui::Color) -> Color {
    Color::from_rgba(color.r, color.g, color.b, color.a)
}

#[allow(dead_code)]
pub fn slot_button_element<'a, Message: Clone + 'a>(
    on_press: Message,
    tokens: ThemeTokens,
) -> Element<'a, Message> {
    let _ = tokens;
    button(text(SLOT_BUTTON_LABEL).size(13))
        .on_press(on_press)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui::{ThemeMode, ThemeModeExt};

    #[test]
    fn builds_slot_strip_with_icon_input_switch_button() {
        let tokens = ThemeMode::Dark.tokens();
        let _el: Element<'_, SlotStripMessage> =
            slot_strip_element(false, "hello", SLOT_BUTTON_LABEL, tokens);
        assert_eq!(SLOT_ICON, Icon::Settings);
        assert_eq!(SLOT_TEXT_LABEL, "Shell");
        assert_eq!(SLOT_SWITCH_LABEL, "On");
        assert_eq!(SLOT_BUTTON_LABEL, "Nana");
        assert_eq!(SLOT_INPUT_PLACEHOLDER, "Type…");
    }

    #[test]
    fn builds_slot_button_element() {
        let tokens = ThemeMode::Dark.tokens();
        let _el: Element<'_, ()> = slot_button_element((), tokens);
    }
}
