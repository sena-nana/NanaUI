use super::*;
use iced::widget::column;

impl GalleryState {
    pub(super) fn workspace_gallery(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let tokens = self.theme_tokens();
        let toggle = |label: &'static str, id: RegionId| {
            let expanded = region_expanded(self.workspace.layout(), &id);
            button(text(format!("{}{label}", if expanded { "隐藏" } else { "显示" })).size(12))
                .height(Length::Fixed(ControlSize::Medium.height_in(tokens.metrics)))
                .padding([0.0, UI_METRICS.control_padding_x])
                .on_press(GalleryMessage::Workspace(WorkspaceAction::ToggleRegion(id)))
                .style(button_style(tokens, ButtonKind::Subtle))
        };
        let popup_title_bar = PopupTitleBarFrame::new(
            text("弹窗标题").size(12),
            GalleryMessage::PrimaryAction,
            GalleryMessage::PrimaryAction,
            GalleryMessage::WindowChrome,
            tokens,
        )
        .view();
        let popup = PopupShell::new(
            container(
                column![
                    text("独立弹窗内容").size(13),
                    text("快速创建并管理项目").size(11).color(colors.muted),
                ]
                .spacing(4),
            )
            .center(Length::Fill),
        )
        .title_bar(popup_title_bar)
        .view(tokens);
        container(
            column![
                text("工作区").size(16).color(colors.text),
                container(
                    column![
                        row![
                            toggle("侧栏", RegionId::Resources),
                            toggle("检查器", RegionId::Inspector),
                            toggle("底部面板", RegionId::Diagnostics),
                        ]
                        .spacing(8),
                        text("拖动区域边缘调整尺寸，双击恢复默认值。")
                            .size(12)
                            .color(colors.muted),
                    ]
                    .spacing(10),
                )
                .width(Length::Fill)
                .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
                .style(panel_style(tokens)),
                container(popup)
                    .width(Length::Fixed(360.0))
                    .height(Length::Fixed(150.0))
                    .style(panel_style(tokens)),
            ]
            .spacing(14),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(iced::Padding {
            top: 20.0,
            right: 24.0,
            bottom: 20.0,
            left: 24.0,
        })
        .style(canvas_style(tokens))
        .into()
    }

    pub(super) fn workspace_toolbar(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let tokens = self.theme_tokens();
        container(
            row![
                text("工作区")
                    .size(13)
                    .font(ui_font(iced::font::Weight::Bold)),
                space().width(Length::Fill),
                button(text("恢复默认").size(12))
                    .height(Length::Fixed(ControlSize::Small.height_in(tokens.metrics),))
                    .padding([0.0, UI_METRICS.control_padding_x])
                    .on_press(GalleryMessage::ResetWorkspaceLayout)
                    .style(button_style(tokens, ButtonKind::Text)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .height(Length::Fill)
        .padding([0, 10])
        .style(toolbar_style(colors))
        .into()
    }

    pub(super) fn workspace_inspector(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let tokens = self.theme_tokens();
        let radius = self.appearance.standard_radius().round() as u8;
        container(column![
            section_heading::<GalleryMessage>(
                "检查器",
                Some(
                    button(text("收起").size(11))
                        .height(Length::Fixed(ControlSize::Small.height_in(tokens.metrics),))
                        .padding([0.0, UI_METRICS.compact_control_padding_x])
                        .on_press(GalleryMessage::Workspace(WorkspaceAction::ToggleRegion(
                            RegionId::Inspector
                        ),))
                        .style(button_style(tokens, ButtonKind::Text))
                        .into(),
                ),
                colors,
            ),
            container(
                column![
                    text("标准圆角").size(11).color(colors.muted),
                    slider(
                        AppearanceSettings::MIN_STANDARD_RADIUS
                            ..=AppearanceSettings::MAX_STANDARD_RADIUS,
                        radius,
                        GalleryMessage::SetStandardRadius,
                    )
                    .height(16)
                    .style(slider_style(colors)),
                    text(format!("{radius} px")).size(11).color(colors.accent),
                    toggler(self.appearance.workspace_corners_enabled())
                        .label("主区域圆角")
                        .on_toggle(GalleryMessage::SetWorkspaceCorners)
                        .size(16)
                        .spacing(8)
                        .text_size(13)
                        .style(toggler_style(colors, false)),
                ]
                .spacing(10),
            )
            .padding([0, 12]),
        ])
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    pub(super) fn workspace_bottom(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let tokens = self.theme_tokens();
        container(column![
            section_heading::<GalleryMessage>(
                "底部面板",
                Some(
                    button(text("收起").size(11))
                        .height(Length::Fixed(ControlSize::Small.height_in(tokens.metrics),))
                        .padding([0.0, UI_METRICS.compact_control_padding_x])
                        .on_press(GalleryMessage::Workspace(WorkspaceAction::ToggleRegion(
                            RegionId::Diagnostics
                        ),))
                        .style(button_style(tokens, ButtonKind::Text))
                        .into(),
                ),
                colors,
            ),
            container(
                row![
                    status_indicator(true, 10.0, colors.success),
                    text("布局就绪").size(11).color(colors.muted),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .padding([0, 12]),
        ])
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}
