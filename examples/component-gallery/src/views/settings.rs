use super::*;
use iced::widget::column;

impl GalleryState {
    pub(super) fn settings_content(&self, _colors: Colors) -> Element<'_, GalleryMessage> {
        let content = match self.settings.active_tab().as_str() {
            "workspace" => self.workspace_settings(),
            "about" => self.about_settings(),
            _ => self.appearance_settings(),
        };
        settings_page(
            &self.settings_model,
            &self.settings,
            content,
            self.theme_tokens(),
        )
    }

    pub(super) fn appearance_settings(&self) -> Element<'_, GalleryMessage> {
        AppearanceSection::new(self.theme, &self.appearance, appearance_message)
            .view(self.theme_tokens())
    }

    pub(super) fn workspace_settings(&self) -> Element<'_, GalleryMessage> {
        let tokens = self.theme_tokens();
        SettingsCollapsibleCard::new(
            column![
                text("工作区布局").size(13),
                text("侧边栏宽度与区域可见状态")
                    .size(11)
                    .color(tokens.colors.muted),
            ]
            .spacing(2),
            column![
                text("恢复默认布局会重置当前工作区的区域尺寸与可见状态。")
                    .size(12)
                    .color(tokens.colors.muted),
                button(text("恢复默认").size(12))
                    .height(Length::Fixed(ControlSize::Small.height_in(tokens.metrics),))
                    .padding([0.0, UI_METRICS.control_padding_x])
                    .on_press(GalleryMessage::ResetWorkspaceLayout)
                    .style(button_style(tokens, ButtonKind::Subtle)),
            ]
            .spacing(10),
            self.workspace_settings_expanded,
            GalleryMessage::ToggleWorkspaceSettingsDetails,
        )
        .view(tokens)
    }

    pub(super) fn about_settings(&self) -> Element<'_, GalleryMessage> {
        AboutSection::new(
            AboutMetadata::new("NanaUI Component Gallery", env!("CARGO_PKG_VERSION"))
                .description("Rust 原生 UI 组件库与工作区框架"),
        )
        .view(self.theme_tokens())
    }
}
