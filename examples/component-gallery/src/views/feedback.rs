use super::*;
use iced::widget::column;

impl GalleryState {
    pub(super) fn feedback(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let tokens = self.theme_tokens();
        let control_height = ControlSize::Medium.height_in(tokens.metrics);
        let compact_height = ControlSize::Small.height_in(tokens.metrics);
        let progress = if self.loading { 72.0 } else { 0.0 };
        let tooltip_config = TooltipConfig::default();
        let action_status = match self.context_action {
            Some(ContextAction::Duplicate) => "已复制".to_owned(),
            Some(ContextAction::Rename) => "已重命名".to_owned(),
            Some(ContextAction::Remove) => "已移除".to_owned(),
            None if self.confirmed_actions > 0 => {
                format!("操作已确认 {} 次", self.confirmed_actions)
            }
            None => "等待操作".to_owned(),
        };
        let actions = container(
            column![
                button(
                    text(if self.overlay.contains(&GalleryOverlay::Dialog) {
                        "关闭对话框"
                    } else {
                        "打开对话框"
                    })
                    .size(11),
                )
                .width(Length::Fill)
                .height(Length::Fixed(control_height))
                .padding([0.0, UI_METRICS.control_padding_x])
                .on_press(GalleryMessage::ToggleDialog)
                .style(button_style(colors, ButtonKind::Primary)),
                button(text("更多操作").size(11))
                    .width(Length::Fill)
                    .height(Length::Fixed(control_height))
                    .padding([0.0, UI_METRICS.control_padding_x])
                    .on_press(GalleryMessage::ToggleContextMenu)
                    .style(button_style(colors, ButtonKind::Subtle)),
                button(text("查看图片").size(11))
                    .width(Length::Fill)
                    .height(Length::Fixed(control_height))
                    .padding([0.0, UI_METRICS.control_padding_x])
                    .on_press(GalleryMessage::ToggleImageViewer)
                    .style(button_style(colors, ButtonKind::Subtle)),
                UiPopover::new(
                    UiTooltip::new(
                        container(icon(Icon::About, 13.0, colors.muted))
                            .width(Length::Fixed(compact_height))
                            .height(Length::Fixed(compact_height))
                            .align_x(iced::alignment::Horizontal::Center)
                            .align_y(iced::alignment::Vertical::Center),
                        container(text("查看当前状态").size(11)).padding([4, 7]),
                    )
                    .config(tooltip_config)
                    .view(colors),
                    column![
                        text(format!("当前状态：{action_status}")).size(12),
                        UiButton::label("执行主要操作")
                            .kind(ButtonKind::Primary)
                            .on_press(GalleryMessage::PrimaryAction)
                            .view(colors),
                    ]
                    .spacing(8),
                    self.popover_open,
                    GalleryMessage::TogglePopover,
                    GalleryMessage::ClosePopover,
                    colors,
                )
                .view(),
            ]
            .spacing(8)
            .align_x(iced::alignment::Horizontal::Center),
        )
        .width(Length::Fixed(140.0))
        .height(Length::Fixed(160.0))
        .padding([8, 10])
        .style(panel_style(colors));

        let content = container(
            column![
                text("反馈").size(14).color(colors.text),
                row![
                    container(
                        column![
                            text(if self.loading {
                                "处理中"
                            } else {
                                "已完成"
                            })
                            .size(13),
                            UiProgress::<GalleryMessage>::new(progress, 100.0).view(colors),
                        ]
                        .spacing(8),
                    )
                    .width(Length::Fill)
                    .height(Length::Fixed(160.0))
                    .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x,])
                    .style(panel_style(colors)),
                    actions,
                ]
                .spacing(10)
                .align_y(Alignment::Start),
                container(
                    column![
                        text("日历热力图").size(12).color(colors.muted),
                        UiCalendarHeatmap::new(
                            self.calendar_model(),
                            GalleryMessage::CalendarHeatmap,
                            colors,
                        )
                        .view(),
                        text(
                            self.calendar_active
                                .as_ref()
                                .map_or("移动指针查看日期".to_owned(), |cell| cell
                                    .title
                                    .clone()),
                        )
                        .size(10)
                        .color(colors.muted),
                    ]
                    .spacing(6),
                )
                .width(Length::Fill)
                .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
                .style(panel_style(colors)),
                text(action_status).size(10).color(colors.muted),
            ]
            .spacing(12),
        )
        .padding(iced::Padding {
            top: 16.0,
            right: 16.0,
            bottom: 16.0,
            left: 16.0,
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .style(canvas_style(colors));

        content.into()
    }

    pub(super) fn context_menu(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let (width, height) = self.active_workspace().viewport_geometry().logical_size;
        ContextMenuHost::new(
            self.context_items(),
            AnchoredMenuPosition::new(Point::new(width - 24.0, 112.0))
                .placement(AnchoredMenuPlacement::BottomEnd),
            Size::new(width, height),
            GalleryMessage::ContextMenu,
            colors,
        )
        .search(&self.context_query, true)
        .active_path(&self.context_path)
        .pending(self.menu_confirmation.pending())
        .view()
    }

    pub(super) fn dialog(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        UiConfirmDialog::new(
            "确认操作",
            "确认后将记录一次完整操作。",
            GalleryMessage::ConfirmDialog,
            GalleryMessage::RequestDialogClose(DialogCloseTrigger::CloseButton),
            GalleryMessage::OverlayInteraction,
        )
        .description("此操作会更新当前状态")
        .on_outside(GalleryMessage::RequestDialogClose(
            DialogCloseTrigger::Outside,
        ))
        .size(DialogSize::Default)
        .view(colors)
    }

    pub(super) fn image_viewer(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let preview = container(
            stack![
                container(space())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(move |_theme| {
                        iced::widget::container::Style::default().background(colors.accent_strong)
                    }),
                container(
                    column![
                        text("NANA").size(48).color(colors.accent_text),
                        text("完整组件库").size(14).color(colors.accent_text),
                    ]
                    .spacing(6)
                    .align_x(iced::alignment::Horizontal::Center),
                )
                .center(Length::Fill),
            ]
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill);
        UiImageViewer::new(
            ImageViewerSource::new(preview)
                .name("NanaUI 渲染预览")
                .metadata("预览图 · 1600 × 900"),
            GalleryMessage::RequestImageViewerClose(DialogCloseTrigger::CloseButton),
            GalleryMessage::RequestImageViewerClose(DialogCloseTrigger::Outside),
            GalleryMessage::OverlayInteraction,
            colors,
        )
        .view()
    }
}
