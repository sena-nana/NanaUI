use iced::widget::{
    button, checkbox, column, container, row, scrollable, slider, space, text, text_input,
};
use iced::{Alignment, Element, Length, Subscription};

use crate::geometry::WorkspaceGeometry;
use crate::icons::{Icon, icon, status_indicator};
use crate::layout::{
    RegionId, RegionPlacement, RegionRole, RegionScope, RegionState, WorkspaceLayout,
};
use crate::node_canvas;
use crate::settings::{
    AppearanceSettings, SettingsCard, SettingsModel, SettingsRow, SettingsState, SettingsTab,
    SettingsTabId, settings_page, settings_sidebar as settings_sidebar_view,
};
use crate::shell::{app_shell, section_heading, title_bar};
use crate::sidebar::{
    SidebarFooter, SidebarFooterButton, SidebarFrame, SidebarRow, SidebarRowState, SidebarSection,
    SidebarSectionState,
};
use crate::theme::{Colors, ThemeMode, ThemeTokens, UI_METRICS, ui_font};
use crate::widgets::{
    ButtonKind, button_style, canvas_style, checkbox_style, panel_style, scrollable_style,
    segmented_surface_style, selection_button_style, slider_style, text_input_style, toolbar_style,
    vertical_scrollbar,
};
use crate::workspace::{WorkspaceAction, WorkspaceController, WorkspaceRegions, workspace_view};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutPreset {
    Code,
    Github,
    Live2D,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Navigation {
    Workspace,
    Search,
    Preview,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Document {
    Overview,
    Nodes,
    Preview,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Workspace(WorkspaceAction),
    ConfigureWorkspace(WorkspaceAction),
    ToggleTheme,
    SetTheme(ThemeMode),
    SetStandardRadius(u8),
    ResetAppearance,
    SelectLayout(LayoutPreset),
    SelectNavigation(Navigation),
    BackFromSettings,
    SelectSettingsTab(SettingsTabId),
    SelectDocument(Document),
    ToggleResourceSection,
    SidebarAnimationFrame,
    ResetWorkspaceLayout,
    AddNode,
    RenderPreview,
    ResetGraph,
    SetIntensity(u8),
    TogglePostProcess(bool),
    SearchChanged(String),
}

/// Application state for the runnable NanaUI workspace demo.
///
/// Business state stays in the demo while region layout and interaction are
/// delegated to the reusable workspace framework.
#[derive(Debug, Clone)]
pub struct WorkspaceState {
    theme: ThemeMode,
    appearance: AppearanceSettings,
    workspace: WorkspaceController,
    settings_workspace: WorkspaceController,
    settings_model: SettingsModel,
    settings: SettingsState,
    active_layout: LayoutPreset,
    active_navigation: Navigation,
    return_navigation: Navigation,
    active_document: Document,
    resource_section: SidebarSectionState,
    node_count: u32,
    preview_revision: u32,
    intensity: u8,
    post_process: bool,
    search_query: String,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceState {
    pub fn new() -> Self {
        let settings_model = settings_model();
        let settings = SettingsState::new(&settings_model);
        Self {
            theme: ThemeMode::Dark,
            appearance: AppearanceSettings::default(),
            workspace: WorkspaceController::with_layout(layout_for(LayoutPreset::Code)),
            settings_workspace: WorkspaceController::with_layout(settings_layout()),
            settings_model,
            settings,
            active_layout: LayoutPreset::Code,
            active_navigation: Navigation::Workspace,
            return_navigation: Navigation::Workspace,
            active_document: Document::Overview,
            resource_section: SidebarSectionState::default(),
            node_count: 3,
            preview_revision: 1,
            intensity: 65,
            post_process: true,
            search_query: String::new(),
        }
    }

    pub fn theme_mode(&self) -> ThemeMode {
        self.theme
    }

    pub fn colors(&self) -> Colors {
        self.theme.colors()
    }

    pub fn appearance(&self) -> &AppearanceSettings {
        &self.appearance
    }

    pub(crate) fn theme_tokens(&self) -> ThemeTokens {
        ThemeTokens::new(self.colors(), self.appearance.metrics())
    }

    pub fn layout(&self) -> &WorkspaceLayout {
        self.workspace.layout()
    }

    pub fn layout_preset(&self) -> LayoutPreset {
        self.active_layout
    }

    pub fn layout_title(&self) -> &'static str {
        layout_title(self.active_layout)
    }

    pub(crate) fn sidebar_toggle_message(&self) -> Option<Message> {
        if self.active_navigation == Navigation::Settings {
            return None;
        }
        let collapsed = self
            .workspace
            .layout()
            .region(&RegionId::Resources)
            .is_some_and(RegionState::collapsed_value);
        Some(Message::Workspace(WorkspaceAction::SetRegionCollapsed(
            RegionId::Resources,
            !collapsed,
        )))
    }

    pub(crate) fn sidebar_collapsed(&self) -> bool {
        self.active_navigation != Navigation::Settings
            && self
                .workspace
                .layout()
                .region(&RegionId::Resources)
                .is_some_and(RegionState::collapsed_value)
    }

    pub fn layout_json(&self) -> Result<String, serde_json::Error> {
        self.workspace.layout_json()
    }

    pub fn restore_layout_json(&mut self, value: &str) -> Result<(), serde_json::Error> {
        self.workspace.restore_layout_json(value)
    }

    /// Returns the current logical/physical rectangles for host content views.
    pub fn geometry(
        &self,
        logical_width: f32,
        logical_height: f32,
        scale_factor: f32,
    ) -> WorkspaceGeometry {
        self.workspace
            .geometry(logical_width, logical_height, scale_factor)
    }

    pub fn viewport_geometry(&self) -> WorkspaceGeometry {
        self.workspace.viewport_geometry()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            self.active_workspace()
                .subscription()
                .map(Message::Workspace),
        ];
        if self.active_navigation != Navigation::Settings {
            subscriptions.push(
                self.resource_section
                    .subscription()
                    .map(|_| Message::SidebarAnimationFrame),
            );
        }
        Subscription::batch(subscriptions)
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::Workspace(action) => {
                let synchronize_viewport = matches!(
                    &action,
                    WorkspaceAction::WindowResized { .. }
                        | WorkspaceAction::WindowScaleFactorChanged(_)
                );
                if self.active_navigation == Navigation::Settings {
                    self.settings_workspace.update(action.clone());
                    if synchronize_viewport {
                        self.workspace.update(action);
                    }
                } else {
                    self.workspace.update(action.clone());
                    if synchronize_viewport {
                        self.settings_workspace.update(action);
                    }
                }
            }
            Message::ConfigureWorkspace(action) => {
                self.workspace.update(action);
            }
            Message::ToggleTheme => self.theme = self.theme.toggle(),
            Message::SetTheme(theme) => self.theme = theme,
            Message::SetStandardRadius(radius) => {
                self.appearance.set_standard_radius(f32::from(radius));
            }
            Message::ResetAppearance => {
                self.appearance.reset();
            }
            Message::SelectLayout(layout) => {
                if self.active_layout != layout {
                    self.active_layout = layout;
                    self.active_navigation = Navigation::Workspace;
                    self.active_document = Document::Overview;
                    self.workspace.replace_layout(layout_for(layout));
                }
            }
            Message::SelectNavigation(navigation) => {
                if navigation == Navigation::Settings
                    && self.active_navigation != Navigation::Settings
                {
                    self.return_navigation = self.active_navigation;
                    if let Some(size) = self
                        .workspace
                        .layout()
                        .region(&RegionId::Resources)
                        .and_then(RegionState::size_value)
                    {
                        self.settings_workspace
                            .update(WorkspaceAction::SetRegionSize(RegionId::Resources, size));
                    }
                }
                self.active_navigation = navigation;
            }
            Message::BackFromSettings => self.active_navigation = self.return_navigation,
            Message::SelectSettingsTab(tab) => {
                self.settings.select(&self.settings_model, &tab);
            }
            Message::SelectDocument(document) => {
                self.active_navigation = Navigation::Workspace;
                self.active_document = document;
            }
            Message::ToggleResourceSection => {
                self.resource_section.toggle();
            }
            Message::SidebarAnimationFrame => {}
            Message::ResetWorkspaceLayout => {
                self.workspace
                    .replace_layout(layout_for(self.active_layout));
            }
            Message::AddNode => {
                self.node_count = self.node_count.saturating_add(1);
                self.preview_revision = self.preview_revision.saturating_add(1);
            }
            Message::RenderPreview => {
                self.preview_revision = self.preview_revision.saturating_add(1);
            }
            Message::ResetGraph => {
                self.node_count = 3;
                self.preview_revision = self.preview_revision.saturating_add(1);
            }
            Message::SetIntensity(intensity) => self.intensity = intensity.min(100),
            Message::TogglePostProcess(enabled) => self.post_process = enabled,
            Message::SearchChanged(query) => self.search_query = query,
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let tokens = self.theme_tokens();
        let colors = tokens.colors;
        if self.active_navigation == Navigation::Settings {
            let regions = WorkspaceRegions::new()
                .with_region(RegionId::Resources, self.settings_sidebar())
                .with_region(RegionId::Primary, self.settings_content(colors));
            let workspace = workspace_view(
                &self.settings_workspace,
                regions,
                colors,
                Message::Workspace,
            );
            return app_shell(title_bar(self, tokens), workspace, colors);
        }

        let mut regions = WorkspaceRegions::new()
            .with_region(RegionId::Resources, self.resource_panel(colors))
            .with_region(RegionId::PrimaryToolbar, self.primary_toolbar(colors))
            .with_region(RegionId::Primary, self.primary_content(colors))
            .with_region(RegionId::Inspector, self.inspector(colors));
        if self.layout().region(&RegionId::Diagnostics).is_some() {
            regions = regions.with_region(RegionId::Diagnostics, self.diagnostics(colors));
        }
        let workspace = workspace_view(&self.workspace, regions, colors, Message::Workspace);
        app_shell(title_bar(self, tokens), workspace, colors)
    }

    fn active_workspace(&self) -> &WorkspaceController {
        if self.active_navigation == Navigation::Settings {
            &self.settings_workspace
        } else {
            &self.workspace
        }
    }

    fn resource_panel(&self, colors: Colors) -> Element<'_, Message> {
        let tokens = self.theme_tokens();
        let project_label = if self.active_layout == LayoutPreset::Github {
            "项目总览"
        } else {
            "工作区"
        };
        let mut navigation = SidebarSection::new("项目").count(3);
        for (target, label, row_icon) in [
            (Navigation::Workspace, project_label, Icon::Folder),
            (Navigation::Search, "搜索", Icon::Search),
            (Navigation::Preview, "预览", Icon::Eye),
        ] {
            navigation = navigation.push(
                SidebarRow::new(label)
                    .leading(icon(row_icon, 14.0, colors.muted))
                    .state(if self.active_navigation == target {
                        SidebarRowState::Active
                    } else {
                        SidebarRowState::Idle
                    })
                    .on_select(Message::SelectNavigation(target))
                    .view(tokens),
            );
        }

        let entries = match self.active_layout {
            LayoutPreset::Code => [
                (Document::Overview, "概览", Icon::File),
                (Document::Nodes, "节点", Icon::Nodes),
                (Document::Preview, "实时预览", Icon::Eye),
            ],
            LayoutPreset::Github => [
                (Document::Overview, "概览", Icon::Folder),
                (Document::Nodes, "议题", Icon::Nodes),
                (Document::Preview, "代码", Icon::File),
            ],
            LayoutPreset::Live2D => [
                (Document::Overview, "当前项目", Icon::Folder),
                (Document::Nodes, "模型参数", Icon::Nodes),
                (Document::Preview, "实时预览", Icon::Eye),
            ],
        };
        let heading = if self.active_layout == LayoutPreset::Github {
            "仓库"
        } else {
            "资源"
        };
        let mut section = SidebarSection::new(heading)
            .count(entries.len())
            .expanded(self.resource_section.expanded())
            .animation_progress(self.resource_section.expansion())
            .on_toggle(Message::ToggleResourceSection);
        for (document, title, row_icon) in entries {
            let selected =
                self.active_navigation == Navigation::Workspace && self.active_document == document;
            let trailing = (document == Document::Nodes)
                .then(|| text(self.node_count).size(11).color(colors.faint));
            let mut item = SidebarRow::new(title)
                .leading(icon(
                    row_icon,
                    14.0,
                    if selected { colors.text } else { colors.muted },
                ))
                .state(if selected {
                    SidebarRowState::Active
                } else {
                    SidebarRowState::Idle
                })
                .on_select(Message::SelectDocument(document));
            if let Some(trailing) = trailing {
                item = item.trailing(trailing);
            }
            section = section.push(item.view(tokens));
        }

        let footer = SidebarFooter::new()
            .push(
                SidebarFooterButton::new("设置", Icon::Settings)
                    .selected(false)
                    .on_press(Message::SelectNavigation(Navigation::Settings))
                    .view(tokens),
            )
            .view(colors);
        SidebarFrame::new(
            column![navigation.view(tokens), section.view(tokens)]
                .spacing(14)
                .width(Length::Fill),
        )
        .footer(footer)
        .view(colors)
    }

    fn primary_toolbar(&self, colors: Colors) -> Element<'_, Message> {
        let tokens = self.theme_tokens();
        let resource_action = if self
            .layout()
            .region(&RegionId::Resources)
            .expect("standard resources region")
            .collapsed_value()
        {
            button(text("显示资源").size(14))
                .on_press(Message::Workspace(WorkspaceAction::ToggleRegion(
                    RegionId::Resources,
                )))
                .height(Length::Fixed(UI_METRICS.compact_control_height))
                .padding([0.0, UI_METRICS.control_padding_x])
                .style(button_style(tokens, ButtonKind::Text))
        } else {
            button(text("隐藏资源").size(14))
                .on_press(Message::Workspace(WorkspaceAction::ToggleRegion(
                    RegionId::Resources,
                )))
                .height(Length::Fixed(UI_METRICS.compact_control_height))
                .padding([0.0, UI_METRICS.control_padding_x])
                .style(button_style(tokens, ButtonKind::Text))
        };
        let inspector_action = if self
            .layout()
            .region(&RegionId::Inspector)
            .expect("standard inspector region")
            .collapsed_value()
        {
            button(text("显示检查器").size(14))
                .on_press(Message::Workspace(WorkspaceAction::ToggleRegion(
                    RegionId::Inspector,
                )))
                .height(Length::Fixed(UI_METRICS.compact_control_height))
                .padding([0.0, UI_METRICS.control_padding_x])
                .style(button_style(tokens, ButtonKind::Text))
        } else {
            button(text("隐藏检查器").size(14))
                .on_press(Message::Workspace(WorkspaceAction::ToggleRegion(
                    RegionId::Inspector,
                )))
                .height(Length::Fixed(UI_METRICS.compact_control_height))
                .padding([0.0, UI_METRICS.control_padding_x])
                .style(button_style(tokens, ButtonKind::Text))
        };
        let mut content = row![
            text(layout_title(self.active_layout))
                .size(13)
                .font(ui_font(iced::font::Weight::Bold))
                .color(colors.text),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        content = content
            .push(space().width(Length::Fill))
            .push(resource_action)
            .push(inspector_action);
        if let Some(diagnostics) = self.layout().region(&RegionId::Diagnostics) {
            let diagnostics_action = button(
                text(if diagnostics.collapsed_value() {
                    "显示底部面板"
                } else {
                    "隐藏底部面板"
                })
                .size(14),
            )
            .on_press(Message::Workspace(WorkspaceAction::ToggleRegion(
                RegionId::Diagnostics,
            )))
            .height(Length::Fixed(UI_METRICS.compact_control_height))
            .padding([0.0, UI_METRICS.control_padding_x])
            .style(button_style(tokens, ButtonKind::Text));
            content = content.push(diagnostics_action);
        }

        container(content)
            .height(Length::Fill)
            .padding([0, 10])
            .style(toolbar_style(colors))
            .into()
    }

    fn primary_content(&self, colors: Colors) -> Element<'_, Message> {
        let tokens = self.theme_tokens();
        let tabs = row![
            self.tab_button("概览", Document::Overview),
            self.tab_button("节点", Document::Nodes),
            self.tab_button("预览", Document::Preview),
        ]
        .spacing(4);

        let content = match self.active_navigation {
            Navigation::Workspace => match self.active_document {
                Document::Overview => self.overview(colors),
                Document::Nodes => self.nodes(colors),
                Document::Preview => self.preview(colors),
            },
            Navigation::Search => self.search_surface(colors),
            Navigation::Preview => self.preview(colors),
            Navigation::Settings => self.settings_content(colors),
        };

        if self.active_navigation == Navigation::Workspace {
            container(column![container(tabs).padding([10, 24]), content,])
                .width(Length::Fill)
                .height(Length::Fill)
                .style(canvas_style(tokens))
                .into()
        } else {
            content
        }
    }

    fn search_surface(&self, colors: Colors) -> Element<'_, Message> {
        let tokens = self.theme_tokens();
        let query = self.search_query.trim().to_lowercase();
        let resources = ["概览", "节点", "实时预览"];
        let mut results = column![].spacing(6);
        let mut has_results = false;
        for title in resources {
            if query.is_empty() || title.to_lowercase().contains(&query) {
                has_results = true;
                results = results.push(
                    container(
                        row![
                            status_indicator(true, 10.0, colors.accent),
                            text(title).size(13),
                            space().width(Length::Fill),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    )
                    .padding([10, 12])
                    .style(panel_style(tokens)),
                );
            }
        }
        if !has_results {
            results = results.push(text("没有匹配项").size(12).color(colors.muted));
        }

        container(
            column![
                text("搜索").size(16).color(colors.text),
                text_input("搜索资源或节点", &self.search_query)
                    .on_input(Message::SearchChanged)
                    .padding([UI_METRICS.field_padding_y, UI_METRICS.field_padding_x,])
                    .size(13)
                    .width(Length::Fill)
                    .style(text_input_style(tokens, false)),
                scrollable(results)
                    .direction(vertical_scrollbar())
                    .style(scrollable_style(colors))
                    .height(Length::Fill),
            ]
            .spacing(12),
        )
        .padding(iced::Padding {
            top: 20.0,
            right: 24.0,
            bottom: 20.0,
            left: 24.0,
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .style(canvas_style(tokens))
        .into()
    }

    fn settings_sidebar(&self) -> Element<'_, Message> {
        settings_sidebar_view(
            &self.settings_model,
            &self.settings,
            Message::BackFromSettings,
            Message::SelectSettingsTab,
            self.theme_tokens(),
        )
    }

    fn settings_content(&self, colors: Colors) -> Element<'_, Message> {
        let tokens = self.theme_tokens();
        let content = match self.settings.active_tab().as_str() {
            "workspace" => self.workspace_settings(colors),
            "about" => self.about_settings(colors),
            _ => self.appearance_settings(colors),
        };
        settings_page(&self.settings_model, &self.settings, content, tokens)
    }

    fn appearance_settings(&self, colors: Colors) -> Element<'_, Message> {
        let tokens = self.theme_tokens();
        let theme_control = container(
            row![
                button(text("暗色").size(13))
                    .height(Length::Fixed(UI_METRICS.compact_control_height))
                    .padding([0.0, UI_METRICS.selection_padding_x])
                    .on_press(Message::SetTheme(ThemeMode::Dark))
                    .style(selection_button_style(
                        tokens,
                        self.theme == ThemeMode::Dark,
                    )),
                button(text("浅色").size(13))
                    .height(Length::Fixed(UI_METRICS.compact_control_height))
                    .padding([0.0, UI_METRICS.selection_padding_x])
                    .on_press(Message::SetTheme(ThemeMode::Light))
                    .style(selection_button_style(
                        tokens,
                        self.theme == ThemeMode::Light,
                    )),
            ]
            .spacing(2),
        )
        .height(Length::Fixed(UI_METRICS.selection_height))
        .padding(2)
        .style(segmented_surface_style(tokens));
        let theme_card = SettingsCard::new(
            "主题",
            SettingsRow::new("配色", theme_control)
                .hint("选择应用配色，修改会立即应用。")
                .first_in_group()
                .last_in_group()
                .view(tokens),
        )
        .view(tokens);

        let standard_radius = self.appearance.standard_radius().round() as u8;
        let radius_control = row![
            slider(
                AppearanceSettings::MIN_STANDARD_RADIUS..=AppearanceSettings::MAX_STANDARD_RADIUS,
                standard_radius,
                Message::SetStandardRadius,
            )
            .width(Length::Fixed(180.0))
            .height(16)
            .style(slider_style(colors)),
            text(format!("{standard_radius} px"))
                .size(11)
                .color(colors.muted)
                .width(Length::Fixed(36.0)),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        let radius_rows = column![
            SettingsRow::new("标准圆角", radius_control)
                .hint("统一调整控件、列表、卡片与页面圆角。")
                .first_in_group()
                .divided(true)
                .view(tokens),
            SettingsRow::new(
                "默认样式",
                button(text("恢复默认圆角").size(12))
                    .height(Length::Fixed(UI_METRICS.compact_control_height))
                    .padding([0.0, UI_METRICS.control_padding_x])
                    .on_press(Message::ResetAppearance)
                    .style(button_style(tokens, ButtonKind::Subtle)),
            )
            .hint("恢复 NanaUI 的默认圆角大小。")
            .last_in_group()
            .view(tokens),
        ]
        .spacing(0);
        let radius_card = SettingsCard::new("圆角", radius_rows).view(tokens);

        column![theme_card, radius_card].spacing(0).into()
    }

    fn workspace_settings(&self, colors: Colors) -> Element<'_, Message> {
        let tokens = self.theme_tokens();
        let mut rows = column![].spacing(0);
        let visible_regions = [
            (RegionId::Resources, "资源侧栏"),
            (RegionId::Inspector, "检查器"),
            (RegionId::Diagnostics, "底部面板"),
        ]
        .into_iter()
        .filter(|(id, _)| self.layout().region(id).is_some())
        .collect::<Vec<_>>();
        for (index, (id, label)) in visible_regions.into_iter().enumerate() {
            rows = rows.push(self.workspace_region_setting(id, label, index == 0, colors));
        }
        rows = rows.push(
            SettingsRow::new(
                "当前布局",
                button(text("恢复默认布局").size(12))
                    .height(Length::Fixed(UI_METRICS.compact_control_height))
                    .padding([0.0, UI_METRICS.control_padding_x])
                    .on_press(Message::ResetWorkspaceLayout)
                    .style(button_style(tokens, ButtonKind::Subtle)),
            )
            .hint("恢复当前工作区预设的面板尺寸与折叠状态。")
            .last_in_group()
            .view(tokens),
        );
        SettingsCard::new("工作区布局", rows).view(tokens)
    }

    fn workspace_region_setting(
        &self,
        id: RegionId,
        label: &'static str,
        first_in_group: bool,
        colors: Colors,
    ) -> Element<'_, Message> {
        let tokens = self.theme_tokens();
        let region = self.layout().region(&id).expect("registered demo region");
        let visible = !region.collapsed_value() && !region.hidden_value();
        let size = region
            .size_value()
            .map(|size| format!("{size:.0} px"))
            .unwrap_or_else(|| "自动".to_owned());
        let toggle_id = id.clone();
        let controls = row![
            text(size).size(11).color(colors.muted),
            checkbox(visible)
                .label(if visible { "已显示" } else { "已隐藏" })
                .on_toggle(move |visible| Message::ConfigureWorkspace(
                    WorkspaceAction::SetRegionCollapsed(toggle_id.clone(), !visible),
                ))
                .size(16)
                .spacing(6)
                .text_size(12)
                .style(checkbox_style(colors, false)),
            button(text("复位尺寸").size(11))
                .height(Length::Fixed(UI_METRICS.compact_control_height))
                .padding([0.0, UI_METRICS.control_padding_x])
                .on_press(Message::ConfigureWorkspace(
                    WorkspaceAction::ResetRegionSize(id),
                ))
                .style(button_style(tokens, ButtonKind::Ghost)),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        let row = SettingsRow::new(label, controls).hint("显示状态与尺寸由工作区布局持久化。");
        if first_in_group {
            row.first_in_group().view(tokens)
        } else {
            row.view(tokens)
        }
    }

    fn about_settings(&self, colors: Colors) -> Element<'_, Message> {
        let tokens = self.theme_tokens();
        SettingsCard::new(
            "关于",
            column![
                SettingsRow::new(
                    "名称",
                    text(env!("CARGO_PKG_NAME")).size(12).color(colors.muted)
                )
                .first_in_group()
                .view(tokens),
                SettingsRow::new(
                    "版本",
                    text(env!("CARGO_PKG_VERSION")).size(12).color(colors.muted)
                )
                .last_in_group()
                .view(tokens),
            ]
            .spacing(0),
        )
        .view(tokens)
    }

    fn tab_button<'a>(
        &'a self,
        label: &'a str,
        document: Document,
    ) -> iced::widget::Button<'a, Message> {
        button(text(label).size(12))
            .height(Length::Fixed(UI_METRICS.selection_height))
            .padding([0.0, UI_METRICS.selection_padding_x])
            .on_press(Message::SelectDocument(document))
            .style(selection_button_style(
                self.theme_tokens(),
                self.active_document == document,
            ))
    }

    fn overview(&self, colors: Colors) -> Element<'_, Message> {
        let tokens = self.theme_tokens();
        let node_header = row![
            column![
                text("节点画布").size(14).color(colors.text),
                text("当前工作区的节点集合").size(11).color(colors.muted),
            ]
            .spacing(4),
            space().width(Length::Fill),
            button(text("添加节点").size(11))
                .height(Length::Fixed(UI_METRICS.compact_control_height))
                .padding([0.0, UI_METRICS.control_padding_x])
                .on_press(Message::AddNode)
                .style(button_style(tokens, ButtonKind::Primary)),
        ]
        .align_y(Alignment::Center)
        .spacing(8);

        container(
            column![
                node_header,
                container(node_canvas::view(self.node_count, colors))
                    .width(Length::Fill)
                    .height(Length::Fill)
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

    fn nodes(&self, colors: Colors) -> Element<'_, Message> {
        let tokens = self.theme_tokens();
        let mut entries = column![].spacing(6);
        for (index, label) in ["Texture Input", "Color Grade", "Preview Output"]
            .into_iter()
            .enumerate()
        {
            entries = entries.push(
                container(
                    row![
                        text(format!("{:02}", index + 1))
                            .size(11)
                            .color(colors.faint),
                        text(label).size(13),
                        space().width(Length::Fill),
                        status_indicator(true, 10.0, colors.success),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                )
                .padding([10, 12])
                .style(panel_style(tokens)),
            );
        }
        entries = entries.push(
            container(
                row![
                    text("总计").size(11).color(colors.muted),
                    space().width(Length::Fill),
                    text(self.node_count.to_string())
                        .size(12)
                        .color(colors.accent),
                ]
                .align_y(Alignment::Center),
            )
            .padding([10, 12])
            .style(canvas_style(tokens)),
        );

        container(
            scrollable(entries)
                .direction(vertical_scrollbar())
                .style(scrollable_style(colors))
                .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(iced::Padding {
            top: 4.0,
            right: 24.0,
            bottom: 20.0,
            left: 24.0,
        })
        .style(canvas_style(tokens))
        .into()
    }

    fn preview(&self, colors: Colors) -> Element<'_, Message> {
        let tokens = self.theme_tokens();
        let status = format!("第 {} 次更新", self.preview_revision);
        let preview_surface = container(
            column![
                text("实时预览").size(17).color(colors.text),
                text("预览区域").size(12).color(colors.muted),
                text(format!("已更新 · {status}"))
                    .size(11)
                    .color(colors.success),
                button(text("刷新预览").size(11))
                    .height(Length::Fixed(UI_METRICS.compact_control_height))
                    .padding([0.0, UI_METRICS.control_padding_x])
                    .on_press(Message::RenderPreview)
                    .style(button_style(tokens, ButtonKind::Primary)),
            ]
            .spacing(10)
            .align_x(iced::alignment::Horizontal::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(panel_style(tokens));

        container(preview_surface)
            .padding(iced::Padding {
                top: 20.0,
                right: 24.0,
                bottom: 20.0,
                left: 24.0,
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .style(canvas_style(tokens))
            .into()
    }

    fn inspector(&self, colors: Colors) -> Element<'_, Message> {
        let tokens = self.theme_tokens();
        let collapse = button(text("收起").size(11))
            .height(Length::Fixed(UI_METRICS.compact_control_height))
            .padding([0.0, UI_METRICS.compact_control_padding_x])
            .on_press(Message::Workspace(WorkspaceAction::ToggleRegion(
                RegionId::Inspector,
            )))
            .style(button_style(tokens, ButtonKind::Text));

        let controls = column![
            text("当前节点").size(11).color(colors.muted),
            container(row![text("Color Grade").size(13),].align_y(Alignment::Center),)
                .padding([9, 10])
                .style(panel_style(tokens)),
            text("强度").size(11).color(colors.muted),
            slider(0..=100, self.intensity, Message::SetIntensity)
                .height(16)
                .style(slider_style(colors)),
            row![
                text(format!("{}%", self.intensity))
                    .size(11)
                    .color(colors.accent),
                space().width(Length::Fill),
                text("实时").size(10).color(colors.success),
            ]
            .align_y(Alignment::Center),
            checkbox(self.post_process)
                .label("启用后处理")
                .on_toggle(Message::TogglePostProcess)
                .size(16)
                .spacing(8)
                .text_size(13)
                .style(checkbox_style(colors, false)),
            button(text("恢复默认").size(11))
                .width(Length::Fill)
                .height(Length::Fixed(UI_METRICS.control_height))
                .padding([0.0, UI_METRICS.control_padding_x])
                .on_press(Message::ResetGraph)
                .style(button_style(tokens, ButtonKind::Subtle)),
        ]
        .spacing(10);

        container(column![
            section_heading("检查器", Some(collapse.into()), colors),
            scrollable(
                container(controls)
                    .width(Length::Fill)
                    .padding(iced::Padding {
                        top: 0.0,
                        right: 12.0,
                        bottom: 12.0,
                        left: 12.0,
                    })
            )
            .direction(vertical_scrollbar())
            .style(scrollable_style(colors))
            .height(Length::Fill)
        ])
        .width(Length::Fill)
        .height(Length::Fill)
        .clip(true)
        .into()
    }

    fn diagnostics(&self, colors: Colors) -> Element<'_, Message> {
        let tokens = self.theme_tokens();
        let toggle = button(text("收起").size(11))
            .height(Length::Fixed(UI_METRICS.compact_control_height))
            .padding([0.0, UI_METRICS.compact_control_padding_x])
            .on_press(Message::Workspace(WorkspaceAction::ToggleRegion(
                RegionId::Diagnostics,
            )))
            .style(button_style(tokens, ButtonKind::Text));
        let heading = section_heading(
            if self.active_layout == LayoutPreset::Live2D {
                "时间轴"
            } else {
                "控制台"
            },
            Some(toggle.into()),
            colors,
        );

        let rows = if self.active_layout == LayoutPreset::Live2D {
            column![
                status_row("主轨道", format!("{} 个关键帧", self.node_count), colors),
                status_row("预览", format!("第 {} 次", self.preview_revision), colors),
                status_row("强度", format!("{}%", self.intensity), colors),
            ]
        } else {
            column![
                status_row("节点图", format!("{} 个节点", self.node_count), colors),
                status_row(
                    "预览",
                    format!("revision {}", self.preview_revision),
                    colors
                ),
                status_row(
                    "后处理",
                    if self.post_process {
                        "已启用"
                    } else {
                        "已停用"
                    },
                    colors,
                ),
            ]
        }
        .spacing(7);

        container(column![
            heading,
            container(rows).padding(iced::Padding {
                top: 0.0,
                right: 12.0,
                bottom: 12.0,
                left: 12.0,
            }),
        ])
        .height(Length::Fill)
        .width(Length::Fill)
        .into()
    }
}

fn layout_for(preset: LayoutPreset) -> WorkspaceLayout {
    let mut regions = vec![
        RegionState::new(RegionId::Resources, RegionRole::Resources)
            .size(220.0)
            .min_size(180.0)
            .max_size(480.0)
            .collapsible(true)
            .resizable(true),
        RegionState::new(RegionId::PrimaryToolbar, RegionRole::Utility)
            .placement(RegionPlacement::Top)
            .scope(RegionScope::Primary)
            .size(34.0),
        RegionState::new(RegionId::Primary, RegionRole::Primary)
            .min_size(320.0)
            .fill_priority(1),
        RegionState::new(RegionId::Inspector, RegionRole::Inspector)
            .size(280.0)
            .min_size(200.0)
            .max_size(560.0)
            .collapsible(true)
            .resizable(true),
    ];
    if preset != LayoutPreset::Github {
        regions.push(
            RegionState::new(
                RegionId::Diagnostics,
                if preset == LayoutPreset::Live2D {
                    RegionRole::Timeline
                } else {
                    RegionRole::Console
                },
            )
            .placement(RegionPlacement::Bottom)
            .scope(RegionScope::Primary)
            .size(200.0)
            .min_size(96.0)
            .max_size(520.0)
            .collapsible(true)
            .resizable(true),
        );
    }
    WorkspaceLayout::new(regions).expect("demo workspace region ids are unique")
}

fn settings_layout() -> WorkspaceLayout {
    WorkspaceLayout::new([
        RegionState::new(RegionId::Resources, RegionRole::Resources)
            .size(220.0)
            .min_size(180.0)
            .max_size(480.0),
        RegionState::new(RegionId::Primary, RegionRole::Primary)
            .min_size(420.0)
            .fill_priority(1),
    ])
    .expect("settings workspace region ids are unique")
}

fn settings_model() -> SettingsModel {
    SettingsModel::new(
        "appearance",
        [
            SettingsTab::new("appearance", "外观").icon(Icon::Appearance),
            SettingsTab::new("workspace", "工作区").icon(Icon::Workspace),
            SettingsTab::new("about", "关于").icon(Icon::About),
        ],
    )
    .expect("demo settings model is valid")
}

fn layout_title(preset: LayoutPreset) -> &'static str {
    match preset {
        LayoutPreset::Code => "LiliaCode",
        LayoutPreset::Github => "LiliaGithub",
        LayoutPreset::Live2D => "Live2DEditor",
    }
}

fn status_row<'a>(
    label: impl text::IntoFragment<'a>,
    value: impl text::IntoFragment<'a>,
    colors: Colors,
) -> iced::widget::Row<'a, Message> {
    row![
        text(label).size(11),
        space().width(Length::Fill),
        text(value).size(10).color(colors.muted),
    ]
    .spacing(8)
}

#[cfg(test)]
mod tests {
    use super::{LayoutPreset, Message, Navigation, WorkspaceState};
    use crate::geometry::LogicalRect;
    use crate::layout::RegionId;
    use crate::workspace::WorkspaceAction;
    use crate::{SettingsTabId, ThemeMode, UI_METRICS};

    #[test]
    fn shell_state_updates_real_regions_and_theme() {
        let mut state = WorkspaceState::new();
        assert_eq!(state.theme_mode(), ThemeMode::Dark);

        state.update(Message::ToggleTheme);
        assert_eq!(state.theme_mode(), ThemeMode::Light);

        state.update(Message::SelectNavigation(Navigation::Preview));
        state.update(Message::Workspace(WorkspaceAction::ToggleRegion(
            RegionId::Resources,
        )));
        state.update(Message::Workspace(WorkspaceAction::ToggleRegion(
            RegionId::Inspector,
        )));
        state.update(Message::Workspace(WorkspaceAction::ToggleRegion(
            RegionId::Diagnostics,
        )));

        assert_eq!(state.active_navigation, Navigation::Preview);
        assert!(
            state
                .layout()
                .region(&RegionId::Resources)
                .expect("resources")
                .collapsed_value()
        );
        assert!(
            state
                .layout()
                .region(&RegionId::Inspector)
                .expect("inspector")
                .collapsed_value()
        );
        assert!(
            state
                .layout()
                .region(&RegionId::Diagnostics)
                .expect("diagnostics")
                .collapsed_value()
        );
        assert_eq!(state.theme_mode(), ThemeMode::Light);
    }

    #[test]
    fn workspace_resize_messages_update_persisted_layout() {
        let mut state = WorkspaceState::new();
        let initial = state
            .layout()
            .region(&RegionId::Resources)
            .and_then(crate::layout::RegionState::size_value)
            .expect("resources size");

        state.update(Message::Workspace(WorkspaceAction::ResizeStart(
            RegionId::Resources,
        )));
        state.update(Message::Workspace(WorkspaceAction::ResizeMove {
            x: 100.0,
            y: 0.0,
        }));
        state.update(Message::Workspace(WorkspaceAction::ResizeMove {
            x: 140.0,
            y: 0.0,
        }));
        state.update(Message::Workspace(WorkspaceAction::ResizeEnd));

        assert_eq!(
            state
                .layout()
                .region(&RegionId::Resources)
                .and_then(crate::layout::RegionState::size_value),
            Some(initial + 40.0)
        );
        let encoded = state.layout_json().expect("layout serializes");
        let mut restored = WorkspaceState::new();
        restored
            .restore_layout_json(&encoded)
            .expect("layout restores");
        assert_eq!(restored.layout(), state.layout());
    }

    #[test]
    fn adding_and_rendering_change_preview_state() {
        let mut state = WorkspaceState::new();
        state.update(Message::AddNode);
        state.update(Message::RenderPreview);
        state.update(Message::SetIntensity(130));

        assert_eq!(state.intensity, 100);
        assert_eq!(state.node_count, 4);
        assert_eq!(state.preview_revision, 3);
    }

    #[test]
    fn layout_presets_register_different_real_region_sets() {
        let mut state = WorkspaceState::new();
        assert!(state.layout().region(&RegionId::Resources).is_some());
        assert!(state.layout().region(&RegionId::GlobalNavigation).is_none());
        assert!(
            state
                .layout()
                .region(&RegionId::SectionNavigation)
                .is_none()
        );

        state.update(Message::SelectLayout(LayoutPreset::Github));
        assert!(state.layout().region(&RegionId::Resources).is_some());
        assert!(
            state
                .layout()
                .region(&RegionId::SectionNavigation)
                .is_none()
        );
        assert!(state.layout().region(&RegionId::Diagnostics).is_none());

        state.update(Message::SelectLayout(LayoutPreset::Live2D));
        assert!(state.layout().region(&RegionId::GlobalNavigation).is_none());
        assert!(
            state
                .layout()
                .region(&RegionId::SectionNavigation)
                .is_none()
        );
        assert_eq!(
            state
                .layout()
                .region(&RegionId::Diagnostics)
                .expect("studio timeline")
                .role(),
            crate::layout::RegionRole::Timeline
        );
    }

    #[test]
    fn workspace_exposes_host_viewport_geometry() {
        let state = WorkspaceState::new();
        let geometry = state.geometry(1440.0, 900.0, 2.0);

        assert_eq!(geometry.physical_size, (2880, 1800));
        assert_eq!(
            geometry
                .region(&RegionId::Resources)
                .expect("resources")
                .logical
                .width,
            220.0
        );
        assert!(
            geometry
                .region(&RegionId::Primary)
                .expect("primary")
                .logical
                .width
                > 0.0
        );
    }

    #[test]
    fn code_workspace_uses_the_lilia_single_sidebar_matrix() {
        let state = WorkspaceState::new();
        let geometry = state.geometry(1280.0, 720.0, 1.0);
        let rect = |id: &RegionId| {
            geometry
                .region(id)
                .expect("Code preset region is registered")
                .logical
        };

        assert_eq!(
            rect(&RegionId::Resources),
            LogicalRect::new(0.0, 36.0, 220.0, 684.0)
        );
        assert_eq!(
            rect(&RegionId::PrimaryToolbar),
            LogicalRect::new(220.0, 36.0, 780.0, 34.0)
        );
        assert_eq!(
            rect(&RegionId::Primary),
            LogicalRect::new(220.0, 70.0, 780.0, 450.0)
        );
        assert_eq!(
            rect(&RegionId::Inspector),
            LogicalRect::new(1000.0, 36.0, 280.0, 684.0)
        );
        assert_eq!(
            rect(&RegionId::Diagnostics),
            LogicalRect::new(220.0, 520.0, 780.0, 200.0)
        );
    }

    #[test]
    fn settings_workspace_preserves_the_application_layout_and_return_target() {
        let mut state = WorkspaceState::new();
        state.update(Message::ConfigureWorkspace(WorkspaceAction::SetRegionSize(
            RegionId::Resources,
            312.0,
        )));
        state.update(Message::SelectNavigation(Navigation::Search));
        let application_layout = state.layout().clone();

        state.update(Message::SelectNavigation(Navigation::Settings));
        assert_eq!(
            state
                .settings_workspace
                .layout()
                .region(&RegionId::Resources)
                .and_then(crate::layout::RegionState::size_value),
            Some(312.0)
        );
        state.update(Message::Workspace(WorkspaceAction::SetRegionCollapsed(
            RegionId::Resources,
            true,
        )));
        assert!(
            !state
                .settings_workspace
                .layout()
                .region(&RegionId::Resources)
                .expect("settings sidebar")
                .collapsed_value()
        );
        assert_eq!(state.layout(), &application_layout);

        state.update(Message::BackFromSettings);
        assert_eq!(state.active_navigation, Navigation::Search);
        assert_eq!(state.layout(), &application_layout);
    }

    #[test]
    fn settings_actions_update_the_real_workspace_and_sidebar_state() {
        let mut state = WorkspaceState::new();
        state.update(Message::SelectNavigation(Navigation::Settings));
        state.update(Message::SelectSettingsTab(SettingsTabId::from("workspace")));
        state.update(Message::ConfigureWorkspace(
            WorkspaceAction::SetRegionCollapsed(RegionId::Inspector, true),
        ));
        assert!(
            state
                .layout()
                .region(&RegionId::Inspector)
                .expect("inspector")
                .collapsed_value()
        );
        assert_eq!(state.settings.active_tab().as_str(), "workspace");

        assert!(state.resource_section.expanded());
        state.update(Message::ToggleResourceSection);
        assert!(!state.resource_section.expanded());
    }

    #[test]
    fn appearance_radius_changes_survive_navigation_and_can_be_reset() {
        let mut state = WorkspaceState::new();
        state.update(Message::SelectNavigation(Navigation::Settings));
        state.update(Message::SetStandardRadius(12));

        state.update(Message::SelectSettingsTab(SettingsTabId::from("about")));
        state.update(Message::BackFromSettings);
        assert_eq!(state.appearance().standard_radius(), 12.0);
        assert_eq!(state.theme_tokens().metrics.radius_xs, 4.0);
        assert_eq!(state.theme_tokens().metrics.radius_sm, 8.0);
        assert_eq!(state.theme_tokens().metrics.radius_md, 12.0);
        assert_eq!(state.theme_tokens().metrics.radius_lg, 16.0);

        state.update(Message::ResetAppearance);
        assert_eq!(state.appearance().metrics(), UI_METRICS);
    }

    #[test]
    fn viewport_updates_stay_synchronized_between_workspaces() {
        let mut state = WorkspaceState::new();
        state.update(Message::SelectNavigation(Navigation::Settings));
        state.update(Message::Workspace(WorkspaceAction::WindowResized {
            width: 1180.0,
            height: 760.0,
        }));
        state.update(Message::Workspace(
            WorkspaceAction::WindowScaleFactorChanged(2.0),
        ));

        assert_eq!(state.workspace.inline_size(), 1180.0);
        assert_eq!(state.settings_workspace.inline_size(), 1180.0);
        assert_eq!(
            state.workspace.viewport_geometry().physical_size,
            (2360, 1520)
        );
        assert_eq!(
            state.settings_workspace.viewport_geometry().physical_size,
            (2360, 1520)
        );
    }

    #[test]
    fn window_events_update_viewport_geometry_inputs() {
        let mut state = WorkspaceState::new();
        state.update(Message::Workspace(WorkspaceAction::WindowResized {
            width: 1000.0,
            height: 700.0,
        }));
        state.update(Message::Workspace(
            WorkspaceAction::WindowScaleFactorChanged(1.5),
        ));

        let geometry = state.viewport_geometry();
        assert_eq!(geometry.logical_size, (1000.0, 700.0));
        assert_eq!(geometry.physical_size, (1500, 1050));
    }
}
