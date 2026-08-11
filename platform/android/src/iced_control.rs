//! Primary control-slot **Iced widget strip** — Icon + Text + Input + Switch + Button.
//!
//! Not DesktopShell. Host tests prove `nana-ui` control types link. On Android, see
//! [`crate::iced_slot_paint`] for wgpu present + pointer/key into the Primary slot.

use std::borrow::Cow;

use iced::widget::{row, text};
use iced::{Alignment, Element, Length};
use nana_ui::{Button, Icon, Input, Switch, ThemeTokens, icon};

/// Label on the Primary slot Button.
pub const SLOT_BUTTON_LABEL: &str = "Nana";

/// Caption shown beside the slot controls.
pub const SLOT_TEXT_LABEL: &str = "Shell";

/// Leading shell glyph in the control strip (Nana line icon, not Lucide SVG).
pub const SLOT_ICON: Icon = Icon::Settings;

/// Logical size for [`SLOT_ICON`].
pub const SLOT_ICON_SIZE: f32 = 18.0;

/// Switch field label (Nana `Switch` caption).
pub const SLOT_SWITCH_LABEL: &str = "On";

/// Placeholder for the slot [`Input`].
pub const SLOT_INPUT_PLACEHOLDER: &str = "Type…";

/// Messages produced by the Primary slot control strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotStripMessage {
    Pressed,
    Toggle(bool),
    Input(String),
}

/// Build the slot strip: Icon + Text + Input + Switch + Button in one row.
pub fn slot_strip_element<'a>(
    switch_on: bool,
    input_value: impl Into<Cow<'a, str>>,
    button_label: impl Into<Cow<'a, str>>,
    tokens: ThemeTokens,
) -> Element<'a, SlotStripMessage> {
    let leading = icon(SLOT_ICON, SLOT_ICON_SIZE, tokens.colors.muted);
    let caption = text(SLOT_TEXT_LABEL).size(14).color(tokens.colors.text);
    let field = Input::new(SLOT_INPUT_PLACEHOLDER, input_value)
        .on_input(SlotStripMessage::Input)
        .view(tokens);
    let switch = Switch::new(switch_on, SLOT_SWITCH_LABEL)
        .on_toggle(SlotStripMessage::Toggle)
        .view(tokens);
    let button = Button::label(button_label)
        .on_press(SlotStripMessage::Pressed)
        .view(tokens);
    row![leading, caption, field, switch, button]
        .spacing(10)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .into()
}

/// Legacy single-button helper (tests / callers that only need a press target).
#[allow(dead_code)]
pub fn slot_button_element<'a, Message: Clone + 'a>(
    on_press: Message,
    tokens: ThemeTokens,
) -> Element<'a, Message> {
    Button::label(SLOT_BUTTON_LABEL)
        .on_press(on_press)
        .view(tokens)
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
