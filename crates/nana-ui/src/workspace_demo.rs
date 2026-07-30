use iced::widget::{
    button, checkbox, column, container, row, scrollable, slider, space, text, text_input,
};
use iced::{Alignment, Element, Length, Subscription};

use crate::geometry::WorkspaceGeometry;
use crate::layout::{
    RegionId, RegionPlacement, RegionRole, RegionScope, RegionState, WorkspaceLayout,
};
use crate::node_canvas;
use crate::shell::{app_shell, section_heading, title_bar};
use crate::theme::{Colors, ThemeMode};
use crate::widgets::{
    ButtonKind, button_style, canvas_style, checkbox_style, list_item_style, panel_style,
    scrollable_style, slider_style, text_input_style, toolbar_style, vertical_scrollbar,
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
    ToggleTheme,
    SelectLayout(LayoutPreset),
    SelectNavigation(Navigation),
    SelectDocument(Document),
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
    workspace: WorkspaceController,
    active_layout: LayoutPreset,
    active_navigation: Navigation,
    active_document: Document,
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
        Self {
            theme: ThemeMode::Dark,
            workspace: WorkspaceController::with_layout(layout_for(LayoutPreset::Code)),
            active_layout: LayoutPreset::Code,
            active_navigation: Navigation::Workspace,
            active_document: Document::Overview,
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

    pub fn layout(&self) -> &WorkspaceLayout {
        self.workspace.layout()
    }

    pub fn layout_preset(&self) -> LayoutPreset {
        self.active_layout
    }

    pub fn layout_title(&self) -> &'static str {
        layout_title(self.active_layout)
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
        self.workspace.subscription().map(Message::Workspace)
    }

    pub fn update(&mut self, message: Message) {
        match message {
            Message::Workspace(action) => {
                self.workspace.update(action);
            }
            Message::ToggleTheme => self.theme = self.theme.toggle(),
            Message::SelectLayout(layout) => {
                if self.active_layout != layout {
                    self.active_layout = layout;
                    self.active_navigation = Navigation::Workspace;
                    self.active_document = Document::Overview;
                    self.workspace.replace_layout(layout_for(layout));
                }
            }
            Message::SelectNavigation(navigation) => self.active_navigation = navigation,
            Message::SelectDocument(document) => self.active_document = document,
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
        let colors = self.colors();
        let mut regions = WorkspaceRegions::new();
        if self.layout().region(&RegionId::GlobalNavigation).is_some() {
            regions =
                regions.with_region(RegionId::GlobalNavigation, self.global_navigation(colors));
        }
        if self.layout().region(&RegionId::SectionNavigation).is_some() {
            regions =
                regions.with_region(RegionId::SectionNavigation, self.section_navigation(colors));
        }
        regions = regions
            .with_region(RegionId::Resources, self.resource_panel(colors))
            .with_region(RegionId::PrimaryToolbar, self.primary_toolbar(colors))
            .with_region(RegionId::Primary, self.primary_content(colors))
            .with_region(RegionId::Inspector, self.inspector(colors));
        let pull_requests = RegionId::custom("pull-requests");
        if self.layout().region(&pull_requests).is_some() {
            regions = regions.with_region(pull_requests, self.pull_requests(colors));
        }
        if self.layout().region(&RegionId::Diagnostics).is_some() {
            regions = regions.with_region(RegionId::Diagnostics, self.diagnostics(colors));
        }
        let workspace = workspace_view(&self.workspace, regions, colors, Message::Workspace);
        app_shell(title_bar(self, colors), workspace, colors)
    }

    fn global_navigation(&self, colors: Colors) -> Element<'_, Message> {
        let items = [
            (Navigation::Workspace, "W", "工作区"),
            (Navigation::Search, "⌕", "搜索"),
            (Navigation::Preview, "◇", "预览"),
        ];

        let mut rail = column![]
            .spacing(8)
            .align_x(iced::alignment::Horizontal::Center);
        for (navigation, glyph, label) in items {
            let selected = self.active_navigation == navigation;
            rail = rail.push(
                button(
                    column![text(glyph).size(18), text(label).size(9),]
                        .spacing(2)
                        .align_x(iced::alignment::Horizontal::Center),
                )
                .width(Length::Fixed(48.0))
                .height(Length::Fixed(48.0))
                .padding(4)
                .on_press(Message::SelectNavigation(navigation))
                .style(button_style(
                    colors,
                    if selected {
                        ButtonKind::Selected
                    } else {
                        ButtonKind::Text
                    },
                )),
            );
        }

        rail = rail.push(space().height(Length::Fill));
        rail = rail.push(
            button(text("⚙").size(17))
                .width(Length::Fixed(48.0))
                .height(Length::Fixed(42.0))
                .on_press(Message::SelectNavigation(Navigation::Settings))
                .style(button_style(
                    colors,
                    if self.active_navigation == Navigation::Settings {
                        ButtonKind::Selected
                    } else {
                        ButtonKind::Text
                    },
                )),
        );

        container(rail)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding([12, 4])
            .into()
    }

    fn resource_panel(&self, colors: Colors) -> Element<'_, Message> {
        let collapse = button(text("收起").size(11))
            .padding([4, 7])
            .on_press(Message::Workspace(WorkspaceAction::ToggleRegion(
                RegionId::Resources,
            )))
            .style(button_style(colors, ButtonKind::Text));

        let entries = match self.active_layout {
            LayoutPreset::Code => [
                (Document::Overview, "概览"),
                (Document::Nodes, "节点"),
                (Document::Preview, "实时预览"),
            ],
            LayoutPreset::Github => [
                (Document::Overview, "概览"),
                (Document::Nodes, "议题"),
                (Document::Preview, "代码"),
            ],
            LayoutPreset::Live2D => [
                (Document::Overview, "当前项目"),
                (Document::Nodes, "模型参数"),
                (Document::Preview, "实时预览"),
            ],
        };
        let mut list = column![].spacing(4);
        for (document, title) in entries {
            let selected = self.active_document == document;
            list = list.push(
                button(
                    row![
                        text(if selected { "●" } else { "○" })
                            .size(11)
                            .color(if selected {
                                colors.accent
                            } else {
                                colors.faint
                            }),
                        text(title).size(12),
                        space().width(Length::Fill),
                    ]
                    .align_y(Alignment::Center)
                    .spacing(7),
                )
                .width(Length::Fill)
                .height(Length::Fixed(34.0))
                .padding([6, 9])
                .on_press(Message::SelectDocument(document))
                .style(list_item_style(colors, selected)),
            );
        }

        let heading = if self.active_layout == LayoutPreset::Github {
            "仓库"
        } else {
            "资源"
        };
        container(column![
            section_heading(heading, Some(collapse.into()), colors),
            scrollable(list)
                .direction(vertical_scrollbar())
                .style(scrollable_style(colors))
                .height(Length::Fill)
        ])
        .width(Length::Fill)
        .height(Length::Fill)
        .clip(true)
        .into()
    }

    fn section_navigation(&self, colors: Colors) -> Element<'_, Message> {
        let entries = [(Document::Overview, "源代码"), (Document::Nodes, "变更")];
        let mut content = column![section_heading("项目", None, colors)].spacing(4);
        for (document, label) in entries {
            content = content.push(
                button(text(label).size(12))
                    .width(Length::Fill)
                    .height(Length::Fixed(34.0))
                    .padding([6, 10])
                    .align_x(iced::alignment::Horizontal::Left)
                    .on_press(Message::SelectDocument(document))
                    .style(list_item_style(colors, self.active_document == document)),
            );
        }
        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn pull_requests(&self, colors: Colors) -> Element<'_, Message> {
        let entries = [
            (Document::Overview, "等待审查"),
            (Document::Preview, "已合并"),
        ];
        let mut content = column![section_heading("Pull Requests", None, colors)].spacing(4);
        for (document, label) in entries {
            content = content.push(
                button(text(label).size(12))
                    .width(Length::Fill)
                    .height(Length::Fixed(34.0))
                    .padding([6, 10])
                    .align_x(iced::alignment::Horizontal::Left)
                    .on_press(Message::SelectDocument(document))
                    .style(list_item_style(colors, self.active_document == document)),
            );
        }
        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn primary_toolbar(&self, colors: Colors) -> Element<'_, Message> {
        let resource_action = if self
            .layout()
            .region(&RegionId::Resources)
            .expect("standard resources region")
            .collapsed_value()
        {
            button(text("显示资源").size(11))
                .on_press(Message::Workspace(WorkspaceAction::ToggleRegion(
                    RegionId::Resources,
                )))
                .padding([5, 8])
                .style(button_style(colors, ButtonKind::Text))
        } else {
            button(text("隐藏资源").size(11))
                .on_press(Message::Workspace(WorkspaceAction::ToggleRegion(
                    RegionId::Resources,
                )))
                .padding([5, 8])
                .style(button_style(colors, ButtonKind::Text))
        };
        let inspector_action = if self
            .layout()
            .region(&RegionId::Inspector)
            .expect("standard inspector region")
            .collapsed_value()
        {
            button(text("显示检查器").size(11))
                .on_press(Message::Workspace(WorkspaceAction::ToggleRegion(
                    RegionId::Inspector,
                )))
                .padding([5, 8])
                .style(button_style(colors, ButtonKind::Text))
        } else {
            button(text("隐藏检查器").size(11))
                .on_press(Message::Workspace(WorkspaceAction::ToggleRegion(
                    RegionId::Inspector,
                )))
                .padding([5, 8])
                .style(button_style(colors, ButtonKind::Text))
        };
        let mut content = row![
            text(layout_title(self.active_layout))
                .size(13)
                .color(colors.text),
            text("/ 工作区").size(12).color(colors.muted),
        ]
        .spacing(6)
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
                .size(11),
            )
            .on_press(Message::Workspace(WorkspaceAction::ToggleRegion(
                RegionId::Diagnostics,
            )))
            .padding([5, 8])
            .style(button_style(colors, ButtonKind::Text));
            content = content.push(diagnostics_action);
        }

        container(content)
            .height(Length::Fill)
            .padding([0, 12])
            .style(toolbar_style(colors))
            .into()
    }

    fn primary_content(&self, colors: Colors) -> Element<'_, Message> {
        let tabs = row![
            self.tab_button("概览", Document::Overview, colors),
            self.tab_button("节点", Document::Nodes, colors),
            self.tab_button("预览", Document::Preview, colors),
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
            Navigation::Settings => self.settings_surface(colors),
        };

        if self.active_navigation == Navigation::Workspace {
            container(column![container(tabs).padding([10, 24]), content,])
                .width(Length::Fill)
                .height(Length::Fill)
                .style(canvas_style(colors))
                .into()
        } else {
            content
        }
    }

    fn search_surface(&self, colors: Colors) -> Element<'_, Message> {
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
                            text("●").size(10).color(colors.accent),
                            text(title).size(13),
                            space().width(Length::Fill),
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center),
                    )
                    .padding([10, 12])
                    .style(panel_style(colors)),
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
                    .padding([6, 9])
                    .size(13)
                    .width(Length::Fill)
                    .style(text_input_style(colors, false)),
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
        .style(canvas_style(colors))
        .into()
    }

    fn settings_surface(&self, colors: Colors) -> Element<'_, Message> {
        let theme_label = match self.theme {
            ThemeMode::Dark => "深色",
            ThemeMode::Light => "浅色",
        };
        container(
            column![
                text("设置").size(16).color(colors.text),
                container(
                    row![
                        column![
                            text("界面主题").size(13),
                            text(theme_label).size(11).color(colors.muted),
                        ]
                        .spacing(4),
                        space().width(Length::Fill),
                        button(text("切换").size(11))
                            .padding([6, 10])
                            .on_press(Message::ToggleTheme)
                            .style(button_style(colors, ButtonKind::Subtle)),
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                )
                .padding([12, 14])
                .style(panel_style(colors)),
                checkbox(self.post_process)
                    .label("启用后处理")
                    .on_toggle(Message::TogglePostProcess)
                    .size(16)
                    .spacing(8)
                    .text_size(13)
                    .style(checkbox_style(colors, false)),
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
        .style(canvas_style(colors))
        .into()
    }

    fn tab_button<'a>(
        &'a self,
        label: &'a str,
        document: Document,
        colors: Colors,
    ) -> iced::widget::Button<'a, Message> {
        button(text(label).size(12))
            .padding([6, 12])
            .on_press(Message::SelectDocument(document))
            .style(button_style(
                colors,
                if self.active_document == document {
                    ButtonKind::Selected
                } else {
                    ButtonKind::Text
                },
            ))
    }

    fn overview(&self, colors: Colors) -> Element<'_, Message> {
        let node_header = row![
            column![
                text("节点画布").size(14).color(colors.text),
                text("当前工作区的节点集合").size(11).color(colors.muted),
            ]
            .spacing(4),
            space().width(Length::Fill),
            button(text("添加节点").size(11))
                .padding([6, 10])
                .on_press(Message::AddNode)
                .style(button_style(colors, ButtonKind::Primary)),
        ]
        .align_y(Alignment::Center)
        .spacing(8);

        container(
            column![
                node_header,
                container(node_canvas::view(self.node_count, colors))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(panel_style(colors)),
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
        .style(canvas_style(colors))
        .into()
    }

    fn nodes(&self, colors: Colors) -> Element<'_, Message> {
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
                        text("●").size(10).color(colors.success),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                )
                .padding([10, 12])
                .style(panel_style(colors)),
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
            .style(canvas_style(colors)),
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
        .style(canvas_style(colors))
        .into()
    }

    fn preview(&self, colors: Colors) -> Element<'_, Message> {
        let status = format!("第 {} 次更新", self.preview_revision);
        let preview_surface = container(
            column![
                text("实时预览").size(17).color(colors.text),
                text("预览区域").size(12).color(colors.muted),
                text(format!("已更新 · {status}"))
                    .size(11)
                    .color(colors.success),
                button(text("刷新预览").size(11))
                    .padding([6, 10])
                    .on_press(Message::RenderPreview)
                    .style(button_style(colors, ButtonKind::Primary)),
            ]
            .spacing(10)
            .align_x(iced::alignment::Horizontal::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(panel_style(colors));

        container(preview_surface)
            .padding(iced::Padding {
                top: 20.0,
                right: 24.0,
                bottom: 20.0,
                left: 24.0,
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .style(canvas_style(colors))
            .into()
    }

    fn inspector(&self, colors: Colors) -> Element<'_, Message> {
        let collapse = button(text("收起").size(11))
            .padding([4, 7])
            .on_press(Message::Workspace(WorkspaceAction::ToggleRegion(
                RegionId::Inspector,
            )))
            .style(button_style(colors, ButtonKind::Text));

        let controls = column![
            text("当前节点").size(11).color(colors.muted),
            container(row![text("Color Grade").size(13),].align_y(Alignment::Center),)
                .padding([9, 10])
                .style(panel_style(colors)),
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
                .padding([7, 10])
                .on_press(Message::ResetGraph)
                .style(button_style(colors, ButtonKind::Subtle)),
        ]
        .spacing(10);

        container(column![
            section_heading("检查器", Some(collapse.into()), colors),
            scrollable(controls)
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
        let toggle = button(text("收起").size(11))
            .padding([4, 7])
            .on_press(Message::Workspace(WorkspaceAction::ToggleRegion(
                RegionId::Diagnostics,
            )))
            .style(button_style(colors, ButtonKind::Text));
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
    let mut regions = Vec::new();
    if preset != LayoutPreset::Live2D {
        regions.push(
            RegionState::new(RegionId::GlobalNavigation, RegionRole::GlobalNavigation)
                .size(56.0)
                .min_size(44.0)
                .max_size(96.0),
        );
    }
    if preset == LayoutPreset::Code {
        regions.push(
            RegionState::new(RegionId::SectionNavigation, RegionRole::SectionNavigation)
                .size(200.0)
                .min_size(160.0)
                .max_size(360.0),
        );
    }
    regions.push(
        RegionState::new(RegionId::Resources, RegionRole::Resources)
            .size(240.0)
            .min_size(180.0)
            .max_size(520.0)
            .collapsible(true)
            .resizable(true),
    );
    if preset == LayoutPreset::Github {
        regions.push(
            RegionState::new(
                RegionId::custom("pull-requests"),
                RegionRole::SectionNavigation,
            )
            .size(230.0)
            .min_size(180.0)
            .max_size(420.0)
            .resizable(true),
        );
    }
    regions.extend([
        RegionState::new(RegionId::PrimaryToolbar, RegionRole::Utility)
            .placement(RegionPlacement::Top)
            .scope(RegionScope::Primary)
            .size(42.0),
        RegionState::new(RegionId::Primary, RegionRole::Primary)
            .min_size(320.0)
            .fill_priority(1),
        RegionState::new(RegionId::Inspector, RegionRole::Inspector)
            .size(240.0)
            .min_size(200.0)
            .max_size(560.0)
            .collapsible(true)
            .resizable(true),
    ]);
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
    use crate::ThemeMode;
    use crate::layout::RegionId;
    use crate::workspace::WorkspaceAction;

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
        assert!(
            state
                .layout()
                .region(&RegionId::SectionNavigation)
                .is_some()
        );

        state.update(Message::SelectLayout(LayoutPreset::Github));
        assert!(
            state
                .layout()
                .region(&RegionId::custom("pull-requests"))
                .is_some()
        );
        assert!(state.layout().region(&RegionId::Diagnostics).is_none());

        state.update(Message::SelectLayout(LayoutPreset::Live2D));
        assert!(state.layout().region(&RegionId::GlobalNavigation).is_none());
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
            240.0
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
