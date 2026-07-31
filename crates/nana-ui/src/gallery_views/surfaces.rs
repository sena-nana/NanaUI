use super::*;
use iced::widget::column;

impl GalleryState {
    pub(super) fn surfaces(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let selected_view = SurfaceView::from_index(self.surface_selection.selected());
        let card = |title: &'static str, detail: &'static str, kind| {
            container(
                UiCard::new(
                    column![
                        text(title).size(13).color(colors.text),
                        text(detail).size(11).color(colors.muted),
                    ]
                    .spacing(6),
                )
                .kind(kind)
                .view(colors),
            )
            .width(Length::FillPortion(1))
            .height(Length::Fixed(96.0))
        };
        let surface_content = match selected_view {
            SurfaceView::Overview => row![
                card("基础表面", "主工作区内容层", CardKind::Surface),
                card("抬升表面", "侧栏与工具面板", CardKind::Raised),
                card("选中表面", "当前激活的内容", CardKind::Selected),
            ],
            SurfaceView::Cards => {
                let cards_data = [
                    ("默认卡片", "普通内容容器", false),
                    ("交互卡片", "支持选择操作", false),
                    ("禁用卡片", "不可进行操作", true),
                ];
                let mut cards = row![].spacing(10);
                for (index, (title, detail, disabled)) in cards_data.into_iter().enumerate() {
                    let selected = self.selected_surface_card == index;
                    let node = UiInteractiveCard::new(
                        column![
                            text(title).size(13),
                            text(detail).size(11).color(colors.muted),
                        ]
                        .spacing(6),
                    )
                    .selected(selected)
                    .disabled(disabled)
                    .on_select(GalleryMessage::SelectSurfaceCard(index))
                    .view(colors);
                    cards = cards.push(
                        container(node)
                            .width(Length::FillPortion(1))
                            .height(Length::Fixed(96.0)),
                    );
                }
                cards
            }
        }
        .spacing(10);
        let segmented = UiTabs::new(
            selected_view,
            [
                SelectionOption::new(SurfaceView::Overview, "概览"),
                SelectionOption::new(SurfaceView::Cards, "卡片"),
            ],
            GalleryMessage::SelectSurfaceView,
        )
        .view(colors);

        container(
            column![
                text("表面层级").size(14).color(colors.text),
                text("基础、抬升与选中状态").size(11).color(colors.muted),
                container(
                    row![
                        text("表面状态").size(12),
                        space().width(Length::Fill),
                        segmented,
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x,])
                .style(panel_style(colors)),
                surface_content,
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
        .style(canvas_style(colors))
        .into()
    }
}
