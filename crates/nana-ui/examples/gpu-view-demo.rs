use iced::widget::{button, column, container, row, shader, text};
use iced::{Element, Length, Subscription, Task};
use nana_ui::widgets::{button_style, card_style};
use nana_ui::{
    AppTitleBar, ButtonKind, CardKind, GpuView, GpuViewMode, GpuViewPalette, ThemeMode, UI_METRICS,
    WindowChromeEvent, WindowChromeState, custom_title_bar_window, ui_font, ui_font_sources,
};

#[derive(Debug, Clone, Copy, PartialEq)]
enum Message {
    Refresh,
    ToggleTheme,
    WindowChrome(WindowChromeEvent),
}

#[derive(Debug, Default)]
struct GpuViewDemo {
    theme: ThemeMode,
    revision: u32,
    window_chrome: WindowChromeState,
}

impl GpuViewDemo {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Refresh => self.revision = self.revision.saturating_add(1),
            Message::ToggleTheme => self.theme = self.theme.toggle(),
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
        let title_bar = AppTitleBar::new("实时预览", colors)
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
            shader(
                GpuView::new(
                    1,
                    GpuViewPalette {
                        background: colors.background,
                        accent: colors.accent_strong,
                    },
                    self.revision,
                )
                .mode(GpuViewMode::Standalone),
            )
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .clip(true)
        .style(card_style(colors, CardKind::Surface));

        let thumbnail = container(
            shader(GpuView::new(
                2,
                GpuViewPalette {
                    background: colors.surface,
                    accent: colors.accent,
                },
                self.revision.saturating_add(2),
            ))
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fixed(116.0))
        .clip(true)
        .style(card_style(colors, CardKind::Outlined));

        let controls = container(
            column![
                text("预览设置").size(13),
                text(format!("版本 {}", self.revision + 1))
                    .size(12)
                    .color(colors.muted),
                text("缩略预览").size(12).color(colors.muted),
                thumbnail,
                button(text("刷新预览").size(13))
                    .height(Length::Fixed(UI_METRICS.control_height))
                    .padding([0.0, UI_METRICS.control_padding_x])
                    .on_press(Message::Refresh)
                    .style(button_style(colors, ButtonKind::Primary)),
            ]
            .spacing(10),
        )
        .width(Length::Fixed(220.0))
        .height(Length::Fill)
        .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
        .style(card_style(colors, CardKind::Surface));

        container(column![
            title_bar,
            container(row![preview, controls].spacing(10))
                .padding(16)
                .width(Length::Fill)
                .height(Length::Fill)
        ])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| {
            iced::widget::container::Style::default()
                .background(colors.background)
                .color(colors.text)
        })
        .into()
    }
}

fn main() -> iced::Result {
    let mut application =
        iced::application(GpuViewDemo::default, GpuViewDemo::update, GpuViewDemo::view)
            .title("NanaUI GPU View Demo")
            .theme(|state: &GpuViewDemo| state.theme.iced_theme())
            .default_font(ui_font(iced::font::Weight::Normal))
            .subscription(GpuViewDemo::subscription)
            .window(custom_title_bar_window(iced::window::Settings {
                size: iced::Size::new(1100.0, 720.0),
                min_size: Some(iced::Size::new(760.0, 520.0)),
                ..iced::window::Settings::default()
            }))
            .centered();
    for source in ui_font_sources() {
        application = application.font(source);
    }
    application.run()
}
