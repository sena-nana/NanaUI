use iced::widget::{button, column, container, row, space, text};
use iced::{Alignment, Color, Element, Length};
use nana_ui::widgets::button_style;
use nana_ui::{ButtonKind, Colors, ThemeMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Message {
    ToggleTheme,
    TogglePanel,
}

#[derive(Debug, Clone)]
struct TransparentWindowState {
    theme: ThemeMode,
    panel_visible: bool,
}

impl Default for TransparentWindowState {
    fn default() -> Self {
        Self {
            theme: ThemeMode::Dark,
            panel_visible: true,
        }
    }
}

impl TransparentWindowState {
    fn update(&mut self, message: Message) {
        match message {
            Message::ToggleTheme => self.theme = self.theme.toggle(),
            Message::TogglePanel => self.panel_visible = !self.panel_visible,
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let colors = self.theme.colors();
        let title = container(
            row![
                text("NANA").size(12).color(colors.accent),
                text("透明窗口预览").size(15).color(colors.text),
                space().width(Length::Fill),
                button(
                    text(if self.theme == ThemeMode::Dark {
                        "浅色"
                    } else {
                        "深色"
                    })
                    .size(13)
                )
                .height(Length::Fixed(32.0))
                .padding([0, 10])
                .on_press(Message::ToggleTheme)
                .style(button_style(colors, ButtonKind::Text)),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        )
        .padding([0, 16])
        .height(Length::Fixed(44.0))
        .style(panel_style(colors, 0.72));

        let preview = container(
            column![
                text("实时预览").size(18).color(colors.text),
                text("内容保持清晰可见").size(12).color(colors.muted),
                button(
                    text(if self.panel_visible {
                        "隐藏面板"
                    } else {
                        "显示面板"
                    })
                    .size(13)
                )
                .height(Length::Fixed(32.0))
                .padding([0, 10])
                .on_press(Message::TogglePanel)
                .style(button_style(colors, ButtonKind::Primary)),
            ]
            .spacing(12)
            .align_x(Alignment::Center),
        )
        .width(Length::Fixed(360.0))
        .padding(28)
        .style(panel_style(colors, 0.86));

        let body = if self.panel_visible {
            container(preview)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
        } else {
            container(text("面板已隐藏").size(12).color(colors.muted))
                .center_x(Length::Fill)
                .center_y(Length::Fill)
        };

        container(column![title, body])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_theme| iced::widget::container::Style {
                background: Some(Color::TRANSPARENT.into()),
                text_color: Some(colors.text),
                ..Default::default()
            })
            .into()
    }
}

fn panel_style(
    colors: Colors,
    alpha: f32,
) -> impl Fn(&iced::Theme) -> iced::widget::container::Style + 'static {
    move |_theme| iced::widget::container::Style {
        background: Some(
            Color {
                a: alpha,
                ..colors.surface
            }
            .into(),
        ),
        text_color: Some(colors.text),
        border: iced::Border {
            color: colors.border,
            width: 1.0,
            radius: 16.0.into(),
        },
        ..Default::default()
    }
}

fn main() -> iced::Result {
    iced::application(
        TransparentWindowState::default,
        TransparentWindowState::update,
        TransparentWindowState::view,
    )
    .title("NanaUI Transparent Window Demo")
    .theme(|state: &TransparentWindowState| state.theme.iced_theme())
    .transparent(true)
    .window(iced::window::Settings {
        size: iced::Size::new(920.0, 620.0),
        min_size: Some(iced::Size::new(640.0, 420.0)),
        blur: true,
        ..iced::window::Settings::default()
    })
    .centered()
    .run()
}
