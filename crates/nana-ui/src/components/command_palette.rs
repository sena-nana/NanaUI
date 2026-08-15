use std::borrow::Cow;
use std::rc::Rc;

use iced::advanced::widget::{self as advanced_widget, Widget};
use iced::advanced::{Layout, Shell, layout, mouse, overlay, renderer};
use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{
    Alignment, Element, Event, Length, Pixels, Rectangle, Size, Theme, Vector, keyboard, widget,
};

use crate::command::{ActionId, ActionPickerNavigation, action_picker_from_iced_key};
use crate::components::ControlSize;
use crate::components::overlays::Dialog;
use crate::dialog::DialogSize;
use crate::theme::{ThemeTokens, ui_font};
use crate::widgets::{
    ButtonKind, button_style, scrollable_style, text_input_style, vertical_scrollbar,
};
use nana_ui_core::{CommandPaletteEvent, CommandPaletteItem};

const PALETTE_MAX_ROWS: usize = 12;
const PALETTE_ROW_HEIGHT: f32 = 40.0;
pub const COMMAND_PALETTE_INPUT_ID: &str = "nana-ui.command-palette.input";

pub struct CommandPalette<'a, Message> {
    title: Cow<'a, str>,
    placeholder: Cow<'a, str>,
    empty_label: Cow<'a, str>,
    items: Vec<CommandPaletteItem>,
    query: Cow<'a, str>,
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
        items: impl IntoIterator<Item = CommandPaletteItem>,
        query: impl Into<Cow<'a, str>>,
        selected: usize,
        on_event: impl Fn(CommandPaletteEvent) -> Message + 'a,
        theme: impl Into<ThemeTokens>,
    ) -> Self {
        Self {
            title: title.into(),
            placeholder: Cow::Borrowed("搜索操作"),
            empty_label: Cow::Borrowed("没有可用操作"),
            items: items.into_iter().collect(),
            query: query.into(),
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
        let selected_action = self
            .items
            .get(self.selected)
            .map(|item| item.action.clone());
        let search = text_input(self.placeholder, self.query)
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
        let dialog = Dialog::new(self.title, body)
            .size(DialogSize::Wide)
            .on_close(dismiss.clone())
            .on_outside(dismiss)
            .close_hidden(true)
            .view(self.tokens);
        Element::new(CommandPaletteLayer {
            content: dialog,
            selected_action,
            on_event: self.on_event,
        })
    }
}

struct CommandPaletteLayer<'a, Message> {
    content: Element<'a, Message>,
    selected_action: Option<ActionId>,
    on_event: Rc<dyn Fn(CommandPaletteEvent) -> Message + 'a>,
}

impl<Message> Widget<Message, Theme, iced::Renderer> for CommandPaletteLayer<'_, Message>
where
    Message: Clone,
{
    fn tag(&self) -> advanced_widget::tree::Tag {
        self.content.as_widget().tag()
    }

    fn state(&self) -> advanced_widget::tree::State {
        self.content.as_widget().state()
    }

    fn diff(&mut self, tree: &mut advanced_widget::Tree) {
        self.content.as_widget_mut().diff(tree);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut advanced_widget::Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content.as_widget_mut().layout(tree, renderer, limits)
    }

    fn update(
        &mut self,
        tree: &mut advanced_widget::Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let event = match command_palette_key(key_pressed(event), self.selected_action.as_ref()) {
            Some(event) => event,
            None => {
                self.content
                    .as_widget_mut()
                    .update(tree, event, layout, cursor, renderer, shell, viewport);
                return;
            }
        };
        shell.publish((self.on_event)(event));
        shell.capture_event();
    }

    fn draw(
        &self,
        tree: &advanced_widget::Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content
            .as_widget()
            .draw(tree, renderer, theme, style, layout, cursor, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &advanced_widget::Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_widget()
            .mouse_interaction(tree, layout, cursor, viewport, renderer)
    }

    fn operate(
        &mut self,
        tree: &mut advanced_widget::Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn advanced_widget::Operation,
    ) {
        self.content
            .as_widget_mut()
            .operate(tree, layout, renderer, operation);
    }

    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut advanced_widget::Tree,
        layout: Layout<'a>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, iced::Renderer>> {
        self.content
            .as_widget_mut()
            .overlay(tree, layout, renderer, viewport, translation)
    }
}

fn key_pressed(event: &Event) -> Option<&keyboard::Key> {
    let Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) = event else {
        return None;
    };
    Some(key)
}

fn command_palette_key(
    key: Option<&keyboard::Key>,
    selected_action: Option<&ActionId>,
) -> Option<CommandPaletteEvent> {
    let navigation = key.and_then(action_picker_from_iced_key)?;
    match navigation {
        ActionPickerNavigation::Confirm => {
            selected_action.cloned().map(CommandPaletteEvent::Select)
        }
        ActionPickerNavigation::Dismiss => Some(CommandPaletteEvent::Dismiss),
        navigation => Some(CommandPaletteEvent::Navigate(navigation)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_navigation_selects_the_current_action() {
        let action = ActionId::new("workspace.files");
        assert_eq!(
            command_palette_key(
                Some(&keyboard::Key::Named(keyboard::key::Named::Enter)),
                Some(&action),
            ),
            Some(CommandPaletteEvent::Select(action))
        );
        assert_eq!(
            command_palette_key(
                Some(&keyboard::Key::Named(keyboard::key::Named::ArrowDown)),
                None,
            ),
            Some(CommandPaletteEvent::Navigate(ActionPickerNavigation::Next))
        );
    }
}
