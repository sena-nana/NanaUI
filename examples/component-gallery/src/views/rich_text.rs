use super::*;

impl GalleryState {
    #[allow(dead_code)]
    pub(super) fn rich_text_gallery(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let tokens = self.theme_tokens();
        let mut content = iced::widget::column![
            text("原生富文本")
                .size(20)
                .font(ui_font(font::Weight::Semibold))
                .color(colors.text),
            text("CommonMark、数学公式与图表共享同一 Iced/WGPU 渲染路径。")
                .size(12)
                .color(colors.muted),
            self.markdown.view(tokens, GalleryMessage::OpenMarkdownLink),
        ]
        .spacing(14)
        .width(Length::Fill);
        if let Some(link) = &self.opened_markdown_link {
            content = content.push(
                text(format!("已选择链接：{link}"))
                    .size(11)
                    .color(colors.accent),
            );
        }
        container(scrollable(content).direction(vertical_scrollbar()))
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding::from([20, 24]))
            .style(canvas_style(tokens))
            .into()
    }
}
