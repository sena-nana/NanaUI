//! Windowed Iced | Runtime A/B for the workspace-family migration batch.
//!
//! Left = Iced compatibility composer. Right = Runtime Scene via IcedSceneView.
//! Keys: Left/Right or 1-8 select a component, T toggles theme.

use iced::keyboard::{self, Key};
use iced::widget::{column, container, row, text};
use iced::{Element, Event, Length, Size, Subscription};
use nana_ui::compatibility::{
    AppTitleBar as IcedAppTitleBar, DockPanel as IcedDockPanel, PaneChrome as IcedPaneChrome,
    PaneChromeAction as IcedPaneChromeAction, PaneChromeActionKind as IcedPaneChromeActionKind,
    PaneTree as IcedPaneTree, PaneTreeNode as IcedPaneTreeNode,
};
use nana_ui::runtime::{
    AppShell, AppTitleBar, Dock, DockAxis, DockNode, DockPanel, DocumentId, LayoutViewport,
    PaneChrome, PaneChromeAction, PaneChromeActionKind, PaneTree, PaneTreeNode, SplitPane,
    Text as RuntimeText, Workspace, WorkspaceRegionSlot,
};
use nana_ui::{
    DockContents, DockController, DockId, DockItemSpec, DockLayout, DockNode as IcedDockNode,
    DockSurfaceId, IcedSceneView, IcedTextShaper, RegionId, SplitAxis, SplitPaneController,
    ThemeMode, ThemeModeExt, ThemeTokens, WorkspaceController, WorkspaceSlots, app_shell,
    dock_workspace, ratio_pane_split, split_pane, ui_font, ui_font_defaults, ui_font_sources,
    workspace_view,
};
use nana_ui_core::{SplitPaneModel, WorkspaceModel};
use nana_ui_scene::RuntimeDocument;

const PANEL: Size = Size::new(560.0, 360.0);
const SLOT_INSET: f32 = 8.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Case {
    Workspace,
    Dock,
    DockPanel,
    SplitPane,
    PaneChrome,
    PaneTree,
    AppShell,
    AppTitleBar,
}

impl Case {
    const ALL: [Self; 8] = [
        Self::Workspace,
        Self::Dock,
        Self::DockPanel,
        Self::SplitPane,
        Self::PaneChrome,
        Self::PaneTree,
        Self::AppShell,
        Self::AppTitleBar,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Dock => "dock",
            Self::DockPanel => "dock-panel",
            Self::SplitPane => "split-pane",
            Self::PaneChrome => "pane-chrome",
            Self::PaneTree => "pane-tree",
            Self::AppShell => "app-shell",
            Self::AppTitleBar => "app-title-bar",
        }
    }

    fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|case| *case == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    fn prev(self) -> Self {
        let idx = Self::ALL.iter().position(|case| *case == self).unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Clone, Debug)]
enum Message {
    Next,
    Prev,
    Theme,
    Select(usize),
}

struct App {
    theme: ThemeMode,
    case: Case,
    document: RuntimeDocument,
}

fn main() -> iced::Result {
    let mut application =
        iced::application(|| (App::new(), ui_font_defaults()), App::update, App::view)
            .title("NanaUI workspace-family A/B")
            .theme(|app: &App| app.theme.iced_theme())
            .default_font(ui_font(iced::font::Weight::Normal))
            .subscription(App::subscription)
            .window(iced::window::Settings {
                size: iced::Size::new(1280.0, 720.0),
                min_size: Some(iced::Size::new(960.0, 560.0)),
                ..iced::window::Settings::default()
            })
            .centered();
    for source in ui_font_sources() {
        application = application.font(source);
    }
    application.run()
}

impl App {
    fn new() -> Self {
        let mut app = Self {
            theme: ThemeMode::Dark,
            case: Case::Workspace,
            document: RuntimeDocument::new(DocumentId::new(1).expect("document")),
        };
        let _ = remount(&mut app.document, app.case, app.theme);
        app
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Next => self.case = self.case.next(),
            Message::Prev => self.case = self.case.prev(),
            Message::Theme => {
                self.theme = match self.theme {
                    ThemeMode::Dark => ThemeMode::Light,
                    ThemeMode::Light => ThemeMode::Dark,
                };
            }
            Message::Select(index) => {
                if let Some(case) = Case::ALL.get(index) {
                    self.case = *case;
                }
            }
        }
        self.document = RuntimeDocument::new(DocumentId::new(1).expect("document"));
        let _ = remount(&mut self.document, self.case, self.theme);
    }

    fn subscription(&self) -> Subscription<Message> {
        iced::event::listen_with(|event, _status, _id| match event {
            Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => match key {
                Key::Named(keyboard::key::Named::ArrowRight) => Some(Message::Next),
                Key::Named(keyboard::key::Named::ArrowLeft) => Some(Message::Prev),
                Key::Character(ch) if ch.eq_ignore_ascii_case("t") => Some(Message::Theme),
                Key::Character(ch) => ch.parse::<usize>().ok().and_then(|n| {
                    (1..=8)
                        .contains(&n)
                        .then_some(Message::Select(n.saturating_sub(1)))
                }),
                _ => None,
            },
            _ => None,
        })
    }

    fn view(&self) -> Element<'_, Message> {
        let tokens = self.theme.tokens();
        let colors = tokens.colors;
        let iced_side = panel("Iced", colors, iced_case(self.case, tokens));
        let runtime_side = panel(
            "Runtime",
            colors,
            IcedSceneView::new(self.document.scene(), PANEL).map_or_else(
                |error| {
                    text(format!("Runtime scene failed: {error}"))
                        .size(12)
                        .into()
                },
                Into::into,
            ),
        );
        column![
            text(format!(
                "{}   ←/→ or 1-8 switch   T theme ({})",
                self.case.label(),
                match self.theme {
                    ThemeMode::Dark => "dark",
                    ThemeMode::Light => "light",
                }
            ))
            .size(13)
            .color(colors.muted),
            row![iced_side, runtime_side]
                .spacing(12)
                .height(Length::Fill),
        ]
        .padding(16)
        .spacing(10)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}

fn panel<'a>(
    title: &'a str,
    colors: nana_ui::Colors,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    column![
        text(title).size(12).color(colors.muted),
        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| iced::widget::container::Style::default().background(colors.background)),
    ]
    .spacing(6)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn slot_label<'a>(label: &'a str, tokens: ThemeTokens) -> Element<'a, Message> {
    container(text(label).size(12).color(tokens.colors.text))
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(SLOT_INSET)
        .into()
}

fn runtime_slot_text(value: &str) -> RuntimeText {
    let mut style = nana_ui::runtime::NodeStyle::default();
    {
        let layout = std::sync::Arc::make_mut(&mut style.layout);
        let inset = nana_ui_core::LengthSpec::Px(SLOT_INSET);
        layout.padding_left = Some(inset);
        layout.padding_right = Some(inset);
        layout.padding_top = Some(inset);
        layout.padding_bottom = Some(inset);
    }
    RuntimeText::new(value).style(style)
}

fn iced_case(case: Case, tokens: ThemeTokens) -> Element<'static, Message> {
    match case {
        Case::Workspace => {
            let controller = WorkspaceController::new();
            workspace_view(
                &controller,
                WorkspaceSlots::new(
                    slot_label("Nav", tokens),
                    slot_label("Files", tokens),
                    slot_label("Toolbar", tokens),
                    slot_label("Primary", tokens),
                    slot_label("Inspector", tokens),
                    slot_label("Diagnostics", tokens),
                ),
                tokens,
                |_| Message::Next,
            )
        }
        Case::Dock => {
            let main = IcedDockNode::Split {
                axis: nana_ui::DockAxis::Horizontal,
                ratio: 0.35,
                first: Box::new(IcedDockNode::Tabs {
                    tabs: vec![DockId::from("nav"), DockId::from("files")],
                    active: DockId::from("nav"),
                }),
                second: Box::new(IcedDockNode::Item {
                    id: DockId::from("primary"),
                }),
            };
            let controller = DockController::new(
                "primary",
                [
                    DockItemSpec::new("primary", "Primary").limits(160.0, 120.0),
                    DockItemSpec::new("nav", "Nav").limits(120.0, 80.0),
                    DockItemSpec::new("files", "Files").limits(120.0, 80.0),
                ],
                DockLayout::new(main),
            )
            .expect("dock fixture");
            dock_workspace(
                &controller,
                DockSurfaceId(0),
                DockContents::new()
                    .insert("primary", slot_label("Primary", tokens))
                    .insert("nav", slot_label("Nav", tokens))
                    .insert("files", slot_label("Files", tokens)),
                |_| Message::Next,
                tokens,
            )
        }
        Case::DockPanel => IcedDockPanel::new(
            column![
                text("Inspector").size(12).color(tokens.colors.text),
                text("Selection").size(10).color(tokens.colors.muted),
            ]
            .spacing(4),
        )
        .padding(10)
        .view(tokens),
        Case::SplitPane => {
            let controller = SplitPaneController::new(SplitAxis::Horizontal, 180.0, 80.0, 320.0);
            split_pane(
                &controller,
                slot_label("First", tokens),
                slot_label("Second", tokens),
                |_| Message::Next,
                tokens,
            )
        }
        Case::PaneChrome => IcedPaneChrome::new(
            text("editor.rs").size(12),
            text("Body").size(12).color(tokens.colors.text),
            [IcedPaneChromeAction::new(
                IcedPaneChromeActionKind::CloseItem,
                "关闭",
                Message::Next,
            )],
            tokens,
        )
        .view(),
        Case::PaneTree => IcedPaneTree::new(
            IcedPaneTreeNode::split(
                "root",
                SplitAxis::Horizontal,
                0.4,
                IcedPaneTreeNode::leaf("left"),
                IcedPaneTreeNode::leaf("right"),
            ),
            {
                let tokens = tokens;
                move |id| slot_label(*id, tokens)
            },
            {
                let tokens = tokens;
                move |_, axis, ratio, first, second| {
                    ratio_pane_split(axis, ratio, first, second, tokens)
                }
            },
        )
        .view(),
        Case::AppShell => app_shell(
            IcedAppTitleBar::new("NanaUI", tokens).view(),
            text("Workspace").size(13).color(tokens.colors.text),
            tokens.colors,
        ),
        Case::AppTitleBar => IcedAppTitleBar::new("NanaUI", tokens).view(),
    }
}

fn remount(
    document: &mut RuntimeDocument,
    case: Case,
    theme: ThemeMode,
) -> Result<(), Box<dyn std::error::Error>> {
    let document_id = document.document();
    document.context_mut().set_theme(theme)?;
    let label = |document: &mut RuntimeDocument, value: &str| {
        document
            .context_mut()
            .create_detached_component(document_id, runtime_slot_text(value))
    };
    let bare = |document: &mut RuntimeDocument, value: &str| {
        document
            .context_mut()
            .create_detached_component(document_id, RuntimeText::new(value))
    };
    let target = match case {
        Case::Workspace => {
            let nav = label(document, "Nav")?;
            let files = label(document, "Files")?;
            let toolbar = label(document, "Toolbar")?;
            let primary = label(document, "Primary")?;
            let inspector = label(document, "Inspector")?;
            let diagnostics = label(document, "Diagnostics")?;
            let workspace = document.context_mut().create_component(
                document_id,
                Workspace::from_model(
                    &WorkspaceModel::new(),
                    [
                        WorkspaceRegionSlot::new(RegionId::GlobalNavigation, nav.stable_id()),
                        WorkspaceRegionSlot::new(RegionId::Resources, files.stable_id()),
                        WorkspaceRegionSlot::new(RegionId::PrimaryToolbar, toolbar.stable_id()),
                        WorkspaceRegionSlot::new(RegionId::Primary, primary.stable_id()),
                        WorkspaceRegionSlot::new(RegionId::Inspector, inspector.stable_id()),
                        WorkspaceRegionSlot::new(RegionId::Diagnostics, diagnostics.stable_id()),
                    ],
                ),
            )?;
            document.context_mut().append_child(workspace, nav)?;
            document.context_mut().append_child(workspace, files)?;
            document.context_mut().append_child(workspace, toolbar)?;
            document.context_mut().append_child(workspace, primary)?;
            document.context_mut().append_child(workspace, inspector)?;
            document
                .context_mut()
                .append_child(workspace, diagnostics)?;
            document.context_mut().assemble_workspace(workspace)?;
            workspace.stable_id()
        }
        Case::Dock => {
            let nav = label(document, "Nav")?;
            let files = label(document, "Files")?;
            let primary = label(document, "Primary")?;
            let dock = document.context_mut().create_component(
                document_id,
                Dock::new(DockNode::split(
                    DockAxis::Horizontal,
                    0.35,
                    DockNode::tabs(
                        ["nav", "files"],
                        "nav",
                        [
                            ("nav", Some(nav.stable_id())),
                            ("files", Some(files.stable_id())),
                        ],
                    ),
                    DockNode::item("primary", Some(primary.stable_id())),
                ))
                .title("nav", "Nav")
                .title("files", "Files")
                .title("primary", "Primary"),
            )?;
            document.context_mut().append_child(dock, nav)?;
            document.context_mut().append_child(dock, files)?;
            document.context_mut().append_child(dock, primary)?;
            document.context_mut().assemble_dock(dock)?;
            dock.stable_id()
        }
        Case::DockPanel => {
            let body = bare(document, "Inspector")?;
            let panel = document.context_mut().create_component(
                document_id,
                DockPanel::new().padding(10.0).content(body.stable_id()),
            )?;
            document.context_mut().append_child(panel, body)?;
            panel.stable_id()
        }
        Case::SplitPane => {
            let first = label(document, "First")?;
            let second = label(document, "Second")?;
            let handle = document
                .context_mut()
                .create_detached_component(document_id, RuntimeText::new(""))?;
            let indicator = document
                .context_mut()
                .create_detached_component(document_id, RuntimeText::new(""))?;
            let pane = document.context_mut().create_component(
                document_id,
                SplitPane::from_model(
                    &SplitPaneModel::new(SplitAxis::Horizontal, 180.0, 80.0, 320.0),
                    first.stable_id(),
                    second.stable_id(),
                )
                .handle(handle.stable_id()),
            )?;
            document.context_mut().append_child(pane, first)?;
            document.context_mut().append_child(handle, indicator)?;
            document.context_mut().append_child(pane, handle)?;
            document.context_mut().append_child(pane, second)?;
            document.context_mut().update_component(pane, |_, _| {})?;
            pane.stable_id()
        }
        Case::PaneChrome => {
            let header = document
                .context_mut()
                .create_detached_component(document_id, RuntimeText::new(""))?;
            let tabs = bare(document, "editor.rs")?;
            let body = bare(document, "Body")?;
            let close = bare(document, "关闭")?;
            let chrome = document.context_mut().create_component(
                document_id,
                PaneChrome::new()
                    .header(header.stable_id())
                    .tabs(tabs.stable_id())
                    .body(body.stable_id())
                    .actions([
                        PaneChromeAction::new(PaneChromeActionKind::CloseItem, "关闭")
                            .target(close.stable_id()),
                    ]),
            )?;
            document.context_mut().append_child(chrome, header)?;
            document.context_mut().append_child(header, tabs)?;
            document.context_mut().append_child(header, close)?;
            document.context_mut().append_child(chrome, body)?;
            chrome.stable_id()
        }
        Case::PaneTree => {
            let left = label(document, "left")?;
            let right = label(document, "right")?;
            let tree = document.context_mut().create_component(
                document_id,
                PaneTree::new(PaneTreeNode::split(
                    "root",
                    SplitAxis::Horizontal,
                    0.4,
                    PaneTreeNode::leaf_content("left", left.stable_id()),
                    PaneTreeNode::leaf_content("right", right.stable_id()),
                )),
            )?;
            document.context_mut().append_child(tree, left)?;
            document.context_mut().append_child(tree, right)?;
            tree.stable_id()
        }
        Case::AppShell => {
            let title = document
                .context_mut()
                .create_detached_component(document_id, AppTitleBar::new("NanaUI"))?;
            let body = bare(document, "Workspace")?;
            let shell = document.context_mut().create_component(
                document_id,
                AppShell::new()
                    .title_bar(title.stable_id())
                    .body(body.stable_id()),
            )?;
            document.context_mut().append_child(shell, title)?;
            document.context_mut().append_child(shell, body)?;
            shell.stable_id()
        }
        Case::AppTitleBar => document
            .context_mut()
            .create_component(document_id, AppTitleBar::new("NanaUI"))?
            .stable_id(),
    };
    let _ = target;
    let _ = document.flush(
        LayoutViewport::new(PANEL.width, PANEL.height),
        &mut IcedTextShaper,
    )?;
    Ok(())
}
