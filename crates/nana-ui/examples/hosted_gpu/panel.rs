use iced::widget::{button, column, container, row, shader, space, text};
use iced::{Alignment, Element, Length};
use nana_ui::widgets::{button_style, card_style, toolbar_style};
use nana_ui::{ButtonKind, CardKind, Colors, GpuTextureView, HostTexture, ThemeMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Message {
    Refresh,
    ToggleTheme,
}

#[derive(Debug, Default)]
pub struct DemoPanel {
    theme: ThemeMode,
    revision: u32,
}

impl DemoPanel {
    pub fn update(&mut self, message: Message) -> bool {
        match message {
            Message::Refresh => {
                self.revision = self.revision.saturating_add(1);
                false
            }
            Message::ToggleTheme => {
                self.theme = self.theme.toggle();
                true
            }
        }
    }

    pub fn view(
        &self,
        texture: HostTexture,
        translucent_window: bool,
    ) -> Element<'_, Message, iced::Theme, iced_wgpu::Renderer> {
        let colors = self.colors();
        let title_bar = container(
            row![
                text("NANA").size(12).color(colors.accent),
                space().width(Length::Fill),
                text("实时预览").size(13),
                space().width(Length::Fill),
                button(
                    text(if self.theme == ThemeMode::Dark {
                        "浅色"
                    } else {
                        "深色"
                    })
                    .size(13),
                )
                .height(Length::Fixed(32.0))
                .padding([0, 10])
                .on_press(Message::ToggleTheme)
                .style(button_style(colors, ButtonKind::Text)),
            ]
            .align_y(Alignment::Center),
        )
        .height(Length::Fixed(36.0))
        .padding([0, 14])
        .style(toolbar_style(colors));

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
                    .height(Length::Fixed(32.0))
                    .padding([0, 12])
                    .on_press(Message::Refresh)
                    .style(button_style(colors, ButtonKind::Primary)),
            ]
            .spacing(10),
        )
        .width(Length::Fixed(220.0))
        .height(Length::Fill)
        .padding([14, 16])
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

    pub fn theme(&self) -> iced::Theme {
        self.theme.iced_theme()
    }

    pub fn colors(&self) -> Colors {
        self.theme.colors()
    }

    pub fn revision(&self) -> u32 {
        self.revision
    }

    pub fn is_dark(&self) -> bool {
        self.theme == ThemeMode::Dark
    }
}

pub fn window_background(color: iced::Color, translucent: bool) -> iced::Color {
    if translucent {
        iced::Color::from_rgba(color.r, color.g, color.b, 0.78)
    } else {
        iced::Color::from_rgb(color.r, color.g, color.b)
    }
}
