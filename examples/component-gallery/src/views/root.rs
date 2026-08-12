use super::*;

impl GalleryState {
    pub fn view(&self) -> Element<'_, GalleryMessage> {
        let tokens = self.theme_tokens();
        let colors = tokens.colors;
        let shell = if self.settings_open {
            DesktopShell::new(
                self.title_bar(tokens),
                self.settings_workspace.clone(),
                self.settings_content(colors),
                GalleryMessage::Workspace,
                tokens,
            )
            .region(RegionId::Resources, self.settings_sidebar())
        } else {
            let mut shell = DesktopShell::new(
                self.title_bar(tokens),
                self.workspace.clone(),
                self.gallery_content(colors),
                GalleryMessage::Workspace,
                tokens,
            )
            .region(RegionId::Resources, self.gallery_sidebar(colors));
            if self.section == GallerySection::Workspace {
                shell = shell
                    .region(RegionId::PrimaryToolbar, self.workspace_toolbar(colors))
                    .inspector(self.workspace_inspector(colors))
                    .bottom(self.workspace_bottom(colors));
            }
            shell
        };
        let shell = if self.overlay.contains(&GalleryOverlay::ContextMenu) {
            shell.overlay(self.context_menu(colors))
        } else {
            shell
        };
        let base = shell.view();

        if self.overlay.contains(&GalleryOverlay::Dialog) {
            stack![base, self.dialog(colors)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.overlay.contains(&GalleryOverlay::ImageViewer) {
            stack![base, self.image_viewer(colors)]
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

    pub(super) fn gallery_sidebar(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let tokens = self.theme_tokens();
        let mut section = SidebarSection::new("Gallery").count(6);
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

    pub(super) fn settings_sidebar(&self) -> Element<'_, GalleryMessage> {
        settings_sidebar_view(
            &self.settings_model,
            &self.settings,
            GalleryMessage::BackFromSettings,
            GalleryMessage::SelectSettingsTab,
            self.theme_tokens(),
        )
    }
}
