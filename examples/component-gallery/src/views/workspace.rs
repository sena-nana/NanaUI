use super::*;
use iced::widget::column;

impl GalleryState {
    pub(super) fn workspace_gallery(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let tokens = self.theme_tokens();
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
        let dock_panel = |label: &'static str, hint: &'static str| {
            container(
                column![
                    text(label).size(12).color(colors.text),
                    text(hint).size(10).color(colors.muted),
                ]
                .spacing(5),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(10)
        };
        let dock = dock_workspace(
            &self.dock,
            DockSurfaceId(0),
            DockContents::new()
                .insert(
                    "gallery.editor",
                    dock_panel("Preview · Program", "不可移动的主内容节点"),
                )
                .insert("gallery.scenes", dock_panel("Scene A", "选择当前场景"))
                .insert("gallery.sources", dock_panel("Image", "当前场景实例"))
                .insert(
                    "gallery.properties",
                    dock_panel("Transform", "位置、缩放与旋转"),
                )
                .insert(
                    "gallery.connection",
                    dock_panel("NanaLive", "已连接的 Actor 输入"),
                )
                .insert("gallery.mixer", dock_panel("Master", "音量与静音"))
                .insert("gallery.cue", dock_panel("Cue 01", "确定性执行队列"))
                .insert(
                    "gallery.controls",
                    dock_panel("Controls", "Take 与输出状态"),
                ),
            GalleryMessage::Dock,
            tokens,
        );
        let locked = self.dock.layout().locked;
        let hidden_sources = !self.dock.is_visible(&DockId::from("gallery.sources"));
        let floating_count = self.dock.layout().floating.len();
        container(
            column![
                text("工作区").size(16).color(colors.text),
                container(
                    column![
                        row![
                            button(text(if locked { "解锁 Dock" } else { "锁定 Dock" }).size(11))
                                .height(Length::Fixed(28.0))
                                .padding([0.0, 8.0])
                                .on_press(GalleryMessage::Dock(DockAction::SetLocked(!locked)))
                                .style(button_style(tokens, ButtonKind::Subtle)),
                            button(
                                text(if hidden_sources {
                                    "恢复 Sources"
                                } else {
                                    "隐藏 Sources"
                                })
                                .size(11)
                            )
                            .height(Length::Fixed(28.0))
                            .padding([0.0, 8.0])
                            .on_press(GalleryMessage::Dock(if hidden_sources {
                                DockAction::Show(DockId::from("gallery.sources"))
                            } else {
                                DockAction::Hide(DockId::from("gallery.sources"))
                            }))
                            .style(button_style(tokens, ButtonKind::Subtle)),
                            button(text("重置 Dock").size(11))
                                .height(Length::Fixed(28.0))
                                .padding([0.0, 8.0])
                                .on_press(GalleryMessage::Dock(DockAction::Reset))
                                .style(button_style(tokens, ButtonKind::Subtle)),
                        ]
                        .spacing(8),
                        text(format!(
                            "拖动分隔条调整，双击复位；当前浮窗 {floating_count} 个。"
                        ))
                        .size(12)
                        .color(colors.muted),
                    ]
                    .spacing(10),
                )
                .width(Length::Fill)
                .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
                .style(panel_style(tokens)),
                container(dock)
                    .width(Length::Fill)
                    .height(Length::Fixed(430.0)),
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
