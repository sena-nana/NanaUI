use iced::widget::{button, column, container, row, shader, text};
use iced::{Element, Length};
use nana_ui::ThemeModeExt;
use nana_ui::compatibility::AppTitleBar;
use nana_ui::widgets::{button_style, card_style};
use nana_ui::{
    AppearanceSettings, ButtonKind, CardKind, Colors, GpuTextureView, HostTexture, ThemeMode,
    UI_METRICS, WindowChromeAction, WindowChromeEvent, WindowChromeState,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Message {
    Refresh,
    ToggleTheme,
    WindowChrome(WindowChromeEvent),
}

#[derive(Debug, Default)]
pub struct DemoPanel {
    theme: ThemeMode,
    revision: u32,
    window_chrome: WindowChromeState,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PanelUpdate {
    pub window_action: Option<WindowChromeAction>,
}

impl DemoPanel {
    pub fn update(&mut self, message: Message) -> PanelUpdate {
        match message {
            Message::Refresh => {
                self.revision = self.revision.saturating_add(1);
                PanelUpdate::default()
            }
            Message::ToggleTheme => {
                self.theme = self.theme.toggle();
                PanelUpdate {
                    window_action: None,
                }
            }
            Message::WindowChrome(event) => PanelUpdate {
                window_action: self.window_chrome.update(event),
            },
        }
    }

    pub fn sync_maximized(&mut self, maximized: bool) {
        self.window_chrome.set_maximized(maximized);
    }

    pub fn view(
        &self,
        texture: HostTexture,
        translucent_window: bool,
    ) -> Element<'static, Message, iced::Theme, iced_wgpu::Renderer> {
        let colors = self.colors();
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
            shader(GpuTextureView::new(texture))
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .clip(true)
        .style(card_style(colors, CardKind::Surface));

        let panel = container(
            column![
                text("预览设置").size(13),
                text(format!("预览版本 {}", self.revision + 1))
                    .size(12)
                    .color(colors.muted),
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
            container(row![preview, panel].spacing(10))
                .padding(16)
                .width(Length::Fill)
                .height(Length::Fill),
        ])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_theme| {
            iced::widget::container::Style::default()
                .background(window_background(colors.background, translucent_window))
                .color(colors.text)
        })
        .into()
    }

    pub fn colors(&self) -> Colors {
        self.theme.colors()
    }

    pub fn revision(&self) -> u32 {
        self.revision
    }

    pub fn theme_mode(&self) -> ThemeMode {
        self.theme
    }
}

pub fn window_background(color: iced::Color, translucent: bool) -> iced::Color {
    if translucent {
        iced::Color::from_rgba(
            color.r,
            color.g,
            color.b,
            AppearanceSettings::DEFAULT_BACKDROP_OPACITY,
        )
    } else {
        iced::Color::from_rgb(color.r, color.g, color.b)
    }
}
