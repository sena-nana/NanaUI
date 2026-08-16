use iced::widget::{button, column, container, text};
use iced::{Alignment, Color, Element, Length, Subscription, Task};
use nana_ui::ThemeModeExt;
use nana_ui::compatibility::AppTitleBar;
use nana_ui::widgets::button_style;
use nana_ui::{
    ButtonKind, Colors, ThemeMode, UI_METRICS, WindowChromeEvent, WindowChromeState,
    custom_title_bar_window, ui_font, ui_font_defaults, ui_font_sources,
};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Message {
    ToggleTheme,
    TogglePanel,
    WindowChrome(WindowChromeEvent),
}

#[derive(Debug, Clone)]
struct TransparentWindowState {
    theme: ThemeMode,
    panel_visible: bool,
    window_chrome: WindowChromeState,
}

impl Default for TransparentWindowState {
    fn default() -> Self {
        Self {
            theme: ThemeMode::Dark,
            panel_visible: true,
            window_chrome: WindowChromeState::default(),
        }
    }
}

impl TransparentWindowState {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ToggleTheme => self.theme = self.theme.toggle(),
            Message::TogglePanel => self.panel_visible = !self.panel_visible,
            Message::WindowChrome(event) => {
                return self
                    .window_chrome
                    .update_iced(event)
                    .map(Message::WindowChrome);
            }
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        WindowChromeState::subscription().map(Message::WindowChrome)
    }

    fn view(&self) -> Element<'_, Message> {
        let colors = self.theme.colors();
        let title = AppTitleBar::new("透明窗口预览", colors)
            .leading(text("NANA").size(12).color(colors.accent))
            .trailing(
                button(
                    text(if self.theme == ThemeMode::Dark {
                        "浅色"
                    } else {
                        "深色"
                    })
                    .size(13),
                )
                .height(Length::Fixed(UI_METRICS.control_height))
                .padding([0.0, UI_METRICS.control_padding_x])
                .on_press(Message::ToggleTheme)
                .style(button_style(colors, ButtonKind::Text)),
            )
            .window_chrome(&self.window_chrome, Message::WindowChrome)
            .view();

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
                .height(Length::Fixed(UI_METRICS.control_height))
                .padding([0.0, UI_METRICS.control_padding_x])
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
            radius: UI_METRICS.radius_md.into(),
        },
        ..Default::default()
    }
}

fn main() -> iced::Result {
    let mut application = iced::application(
        || (TransparentWindowState::default(), ui_font_defaults()),
        TransparentWindowState::update,
        TransparentWindowState::view,
    )
    .title("NanaUI Transparent Window Demo")
    .theme(|state: &TransparentWindowState| state.theme.iced_theme())
    .default_font(ui_font(iced::font::Weight::Normal))
    .subscription(TransparentWindowState::subscription)
    .transparent(true)
    .window(custom_title_bar_window(iced::window::Settings {
        size: iced::Size::new(920.0, 620.0),
        min_size: Some(iced::Size::new(640.0, 420.0)),
        blur: true,
        ..iced::window::Settings::default()
    }))
    .centered();
    for source in ui_font_sources() {
        application = application.font(source);
    }
    application.run()
}
