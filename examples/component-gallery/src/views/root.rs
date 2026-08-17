use super::*;
use nana_ui::compatibility::{
    SidebarFooter, SidebarFooterButton, SidebarFrame, SidebarRow, SidebarRowState, SidebarSection,
};

impl GalleryState {
    pub fn view(&self) -> Element<'_, GalleryMessage> {
        let base = if self.settings_open {
            self.settings_runtime_view()
        } else {
            self.gallery_runtime_view()
        };

        if self.overlay.is_open() {
            stack![base, self.overlay_runtime_view()]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            base
        }
    }

    pub(super) fn active_workspace(&self) -> &WorkspaceController {
        if self.settings_open {
            &self.settings_workspace
        } else {
            &self.workspace
        }
    }

    #[allow(dead_code)]
    fn command_palette(&self, tokens: ThemeTokens) -> Element<'_, GalleryMessage> {
        UiCommandPalette::new(
            "命令",
            self.palette_items(),
            self.action_picker.query(),
            self.action_picker.selected(),
            GalleryMessage::CommandPalette,
            tokens,
        )
        .placeholder("搜索命令")
        .view()
    }

    pub(super) fn set_workspace_showcase_visible(&mut self, visible: bool) {
        for id in [
            RegionId::PrimaryToolbar,
            RegionId::Inspector,
            RegionId::Diagnostics,
        ] {
            self.workspace
                .update(WorkspaceAction::SetRegionVisible(id, visible));
        }
    }

    #[allow(dead_code)]
    pub(super) fn title_bar(&self, tokens: ThemeTokens) -> Element<'_, GalleryMessage> {
        let colors = tokens.colors;
        let active_workspace = self.active_workspace();
        let sidebar_collapsed = active_workspace
            .layout()
            .region(&RegionId::Resources)
            .is_some_and(RegionState::collapsed_value);
        let compact_height = ControlSize::Small.height_in(tokens.metrics);
        let sidebar_toggle = button(icon(Icon::Sidebar, 16.0, colors.muted))
            .width(Length::Fixed(compact_height))
            .height(Length::Fixed(compact_height))
            .padding(0)
            .on_press(GalleryMessage::Workspace(WorkspaceAction::ToggleRegion(
                RegionId::Resources,
            )))
            .style(button_style(
                tokens,
                if sidebar_collapsed {
                    ButtonKind::Selected
                } else {
                    ButtonKind::Ghost
                },
            ));
        let theme_icon = match self.theme {
            ThemeMode::Dark => Icon::Appearance,
            ThemeMode::Light => Icon::Moon,
        };
        let context = if self.settings_open {
            "设置"
        } else {
            section_label(self.section)
        };
        let trailing = row![
            text(context).size(11).color(colors.muted),
            button(icon(Icon::Search, 14.0, colors.muted))
                .on_press(GalleryMessage::ToggleCommandPalette)
                .width(Length::Fixed(compact_height))
                .height(Length::Fixed(compact_height))
                .padding(0)
                .style(button_style(tokens, ButtonKind::Text)),
            button(icon(theme_icon, 14.0, colors.accent))
                .on_press(GalleryMessage::ToggleTheme)
                .width(Length::Fixed(compact_height))
                .height(Length::Fixed(compact_height))
                .padding(0)
                .style(button_style(tokens, ButtonKind::Text)),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        AppTitleBar::new("NanaUI Gallery", tokens)
            .leading(sidebar_toggle)
            .trailing(trailing)
            .window_chrome(&self.window_chrome, GalleryMessage::WindowChrome)
            .view()
    }

    #[allow(dead_code)]
    pub(super) fn gallery_sidebar(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let tokens = self.theme_tokens();
        let mut section = SidebarSection::new("Gallery").count(6).tools(
            UiIconButton::new("搜索命令", Icon::Search)
                .on_press(GalleryMessage::ToggleCommandPalette)
                .size(ControlSize::Small)
                .view(tokens),
        );
        for (target, label, row_icon) in [
            (GallerySection::Controls, "控件", Icon::Settings),
            (GallerySection::Surfaces, "表面", Icon::Folder),
            (GallerySection::Feedback, "反馈", Icon::About),
            (GallerySection::RichText, "富文本", Icon::About),
            (GallerySection::Graph, "节点图", Icon::Nodes),
            (GallerySection::Workspace, "工作区", Icon::Workspace),
        ] {
            section = section.push(
                SidebarRow::new(label)
                    .leading(icon(row_icon, 14.0, colors.muted))
                    .state(if self.section == target {
                        SidebarRowState::Active
                    } else {
                        SidebarRowState::Idle
                    })
                    .on_select(GalleryMessage::SelectSection(target))
                    .view(tokens),
            );
        }
        let footer = SidebarFooter::new()
            .push(
                SidebarFooterButton::new("设置", Icon::Settings)
                    .on_press(GalleryMessage::OpenSettings)
                    .view(tokens),
            )
            .view(colors);
        SidebarFrame::new(section.view(tokens))
            .footer(footer)
            .view(colors)
    }

    #[allow(dead_code)]
    pub(super) fn gallery_content(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        match self.section {
            GallerySection::Controls => self.controls(colors),
            GallerySection::Surfaces => self.surfaces(colors),
            GallerySection::Feedback => self.feedback(colors),
            GallerySection::RichText => self.rich_text_gallery(colors),
            GallerySection::Graph => self.graph_gallery(colors),
            GallerySection::Workspace => self.workspace_gallery(colors),
        }
    }
}
