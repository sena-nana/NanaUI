use std::borrow::Cow;
use std::rc::Rc;

use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Element, Length, Pixels, widget};

use crate::command::ActionId;
use crate::components::ControlSize;
use crate::components::overlays::Dialog;
use crate::dialog::DialogSize;
use crate::theme::{ThemeTokens, ui_font};
use crate::widgets::{
    ButtonKind, button_style, scrollable_style, text_input_style, vertical_scrollbar,
};

const PALETTE_MAX_ROWS: usize = 12;
const PALETTE_ROW_HEIGHT: f32 = 40.0;
pub const COMMAND_PALETTE_INPUT_ID: &str = "nana-ui.command-palette.input";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPaletteItem<'a> {
    pub action: ActionId,
    pub label: Cow<'a, str>,
    pub category: Option<Cow<'a, str>>,
    pub shortcut: Option<Cow<'a, str>>,
}

impl<'a> CommandPaletteItem<'a> {
    pub fn new(action: impl Into<ActionId>, label: impl Into<Cow<'a, str>>) -> Self {
        Self {
            action: action.into(),
            label: label.into(),
            category: None,
            shortcut: None,
        }
    }

    pub fn category(mut self, category: impl Into<Cow<'a, str>>) -> Self {
        self.category = Some(category.into());
        self
    }

    pub fn shortcut(mut self, shortcut: impl Into<Cow<'a, str>>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandPaletteEvent {
    Search(String),
    Select(ActionId),
    Dismiss,
    Interaction,
}

pub struct CommandPalette<'a, Message> {
    title: Cow<'a, str>,
    placeholder: Cow<'a, str>,
    empty_label: Cow<'a, str>,
    items: Vec<CommandPaletteItem<'a>>,
    query: &'a str,
    selected: usize,
    input_id: widget::Id,
    on_event: Rc<dyn Fn(CommandPaletteEvent) -> Message + 'a>,
    tokens: ThemeTokens,
}

impl<'a, Message> CommandPalette<'a, Message>
where
    Message: Clone + 'a,
{
    pub fn new(
        title: impl Into<Cow<'a, str>>,
        items: impl IntoIterator<Item = CommandPaletteItem<'a>>,
        query: &'a str,
        selected: usize,
        on_event: impl Fn(CommandPaletteEvent) -> Message + 'a,
        theme: impl Into<ThemeTokens>,
    ) -> Self {
        Self {
            title: title.into(),
            placeholder: Cow::Borrowed("搜索操作"),
            empty_label: Cow::Borrowed("没有可用操作"),
            items: items.into_iter().collect(),
            query,
            selected,
            input_id: widget::Id::new(COMMAND_PALETTE_INPUT_ID),
            on_event: Rc::new(on_event),
            tokens: theme.into(),
        }
    }

    pub fn placeholder(mut self, placeholder: impl Into<Cow<'a, str>>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn empty_label(mut self, label: impl Into<Cow<'a, str>>) -> Self {
        self.empty_label = label.into();
        self
    }

    pub fn input_id(mut self, id: impl Into<widget::Id>) -> Self {
        self.input_id = id.into();
        self
    }

    pub fn view(self) -> Element<'a, Message> {
        let colors = self.tokens.colors;
        let input_size = ControlSize::Medium;
        let search = text_input(&self.placeholder, self.query)
            .id(self.input_id)
            .on_input({
                let on_event = Rc::clone(&self.on_event);
                move |query| on_event(CommandPaletteEvent::Search(query))
            })
            .padding([
                input_size.vertical_padding(self.tokens.metrics),
                input_size.padding_x(),
            ])
            .size(input_size.text_size())
            .line_height(iced::widget::text::LineHeight::Absolute(Pixels(
                input_size.line_height(),
            )))
            .style(text_input_style(self.tokens, false));

        let list: Element<'a, Message> = if self.items.is_empty() {
            container(text(self.empty_label).size(12).color(colors.muted))
                .width(Length::Fill)
                .height(Length::Fixed(PALETTE_ROW_HEIGHT))
                .center(Length::Fill)
                .into()
        } else {
            let mut rows = column![].spacing(1);
            let item_count = self.items.len();
            for (index, item) in self.items.into_iter().enumerate() {
                let mut label = column![
                    text(item.label)
                        .size(12)
                        .font(ui_font(iced::font::Weight::Medium))
                        .color(colors.text)
                ]
                .spacing(1)
                .width(Length::Fill);
                if let Some(category) = item.category {
                    label = label.push(text(category).size(10).color(colors.muted));
                }
                let mut content = row![label]
                    .spacing(10)
                    .align_y(Alignment::Center)
                    .width(Length::Fill);
                if let Some(shortcut) = item.shortcut {
                    content = content.push(text(shortcut).size(10).color(colors.muted));
                }
                rows = rows.push(
                    button(content)
                        .width(Length::Fill)
                        .height(Length::Fixed(PALETTE_ROW_HEIGHT))
                        .padding([0.0, 10.0])
                        .align_x(iced::alignment::Horizontal::Left)
                        .on_press((self.on_event)(CommandPaletteEvent::Select(item.action)))
                        .style(button_style(
                            self.tokens,
                            if index == self.selected {
                                ButtonKind::Selected
                            } else {
                                ButtonKind::Ghost
                            },
                        )),
                );
            }
            let visible_rows = item_count.clamp(1, PALETTE_MAX_ROWS);
            scrollable(rows)
                .direction(vertical_scrollbar())
                .style(scrollable_style(colors))
                .height(Length::Fixed(PALETTE_ROW_HEIGHT * visible_rows as f32))
                .into()
        };

        let body = column![search, list].spacing(8).width(Length::Fill);
        let dismiss = (self.on_event)(CommandPaletteEvent::Dismiss);
        let interaction = (self.on_event)(CommandPaletteEvent::Interaction);
        Dialog::new(self.title, body)
            .size(DialogSize::Wide)
            .on_close(dismiss.clone())
            .on_outside(dismiss)
            .on_interaction(interaction)
            .close_hidden(true)
            .view(self.tokens)
    }
}
