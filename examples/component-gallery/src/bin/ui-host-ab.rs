//! Windowed Runtime fixtures painted by SceneWgpuPainter.
//!
//! The Nana Scene host (`run_runtime_scene`) is the only paint path. Keys:
//! Left/Right or 1-9 select a component, T toggles theme.

use nana_ui::runtime::{
    AppShell, AppTitleBar, CalendarHeatmap as RuntimeCalendarHeatmap,
    CalendarHeatmapDatum as RuntimeCalendarHeatmapDatum, Dock, DockAxis, DockNode, DockPanel,
    DocumentId, FrameworkError, GraphCanvas as RuntimeGraphCanvas, PaneChrome, PaneChromeAction,
    PaneChromeActionKind, PaneTree, PaneTreeNode, SettingsPage, SplitPane, Text as RuntimeText,
    Workspace, WorkspaceRegionSlot,
};
use nana_ui::{
    GraphEdge, GraphEndpoint, GraphModel, GraphNode, GraphPoint, GraphPort, GraphPortKind,
    GraphPortSide, GraphSelection, GraphSize, GraphViewport, RegionId, RuntimeProgram,
    RuntimeProgramContext, RuntimeProgramUpdate, RuntimeRedraw, RuntimeWindowSettings,
    SettingsModel, SettingsState, SettingsTab, SplitAxis, ThemeMode, run_runtime_scene,
};
use nana_ui_core::{SplitPaneModel, WorkspaceModel};
use nana_ui_platform::{InputEvent, WindowCommand, WindowId};
use nana_ui_scene::RuntimeDocument;

const SLOT_INSET: f32 = 8.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Case {
    GraphCanvas,
    Workspace,
    Dock,
    DockPanel,
    SplitPane,
    PaneChrome,
    PaneTree,
    AppShell,
    AppTitleBar,
    SettingsPage,
    Calendar,
}

impl Case {
    const ALL: [Self; 11] = [
        Self::GraphCanvas,
        Self::Workspace,
        Self::Dock,
        Self::DockPanel,
        Self::SplitPane,
        Self::PaneChrome,
        Self::PaneTree,
        Self::AppShell,
        Self::AppTitleBar,
        Self::SettingsPage,
        Self::Calendar,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::GraphCanvas => "graph-canvas",
            Self::Workspace => "workspace",
            Self::Dock => "dock",
            Self::DockPanel => "dock-panel",
            Self::SplitPane => "split-pane",
            Self::PaneChrome => "pane-chrome",
            Self::PaneTree => "pane-tree",
            Self::AppShell => "app-shell",
            Self::AppTitleBar => "app-title-bar",
            Self::SettingsPage => "settings-page",
            Self::Calendar => "calendar",
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

struct App {
    theme: ThemeMode,
    case: Case,
    graph: GraphModel,
    graph_viewport: GraphViewport,
    graph_selection: Option<GraphSelection>,
    document: RuntimeDocument,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_runtime_scene::<App>(
        RuntimeWindowSettings::new("NanaUI Runtime SceneWgpuPainter")
            .initial_size(1280.0, 720.0)
            .minimum_size(960.0, 560.0)
            .system_caption(true),
    )?;
    Ok(())
}

impl RuntimeProgram for App {
    type Message = ();
    type Error = FrameworkError;

    fn initialize(
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        let graph = ab_graph();
        let graph_viewport = graph
            .bounds()
            .map(|bounds| GraphViewport::fit(bounds, GraphSize::new(1248.0, 680.0), 28.0))
            .unwrap_or_default();
        let mut app = Self {
            theme: ThemeMode::Dark,
            case: Case::SettingsPage,
            graph,
            graph_viewport,
            graph_selection: None,
            document: RuntimeDocument::new(DocumentId::new(1).expect("document")),
        };
        app.remount_case()?;
        Ok((app, Vec::new()))
    }

    fn with_document<R>(
        &self,
        id: WindowId,
        f: impl FnOnce(&RuntimeDocument) -> R,
    ) -> Result<Option<R>, nana_ui::DocumentAccessError> {
        let document = { (id == WindowId::PRIMARY).then_some(&self.document) };
        Ok(document.map(f))
    }

    fn with_document_mut<R>(
        &mut self,
        id: WindowId,
        f: impl FnOnce(&mut RuntimeDocument) -> R,
    ) -> Result<Option<R>, nana_ui::DocumentAccessError> {
        let document = { (id == WindowId::PRIMARY).then_some(&mut self.document) };
        Ok(document.map(f))
    }

    fn update(
        &mut self,
        _message: Self::Message,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        RuntimeProgramUpdate::default()
    }

    fn theme_mode(&self) -> ThemeMode {
        self.theme
    }

    fn input_event(
        &mut self,
        id: WindowId,
        event: &InputEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        let InputEvent::Keyboard {
            pressed: true, key, ..
        } = event
        else {
            return Ok(RuntimeProgramUpdate::redraw(id));
        };
        let changed = match key.as_str() {
            "ArrowRight" | "]" => {
                self.case = self.case.next();
                true
            }
            "ArrowLeft" | "[" => {
                self.case = self.case.prev();
                true
            }
            "t" | "T" => {
                self.theme = match self.theme {
                    ThemeMode::Dark => ThemeMode::Light,
                    ThemeMode::Light => ThemeMode::Dark,
                };
                true
            }
            digit if digit.len() == 1 && digit.as_bytes()[0].is_ascii_digit() => {
                let n = digit.parse::<usize>().unwrap_or(0);
                if (1..=9).contains(&n) {
                    if let Some(case) = Case::ALL.get(n.saturating_sub(1)) {
                        self.case = *case;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            _ => false,
        };
        if !changed {
            return Ok(RuntimeProgramUpdate::redraw(id));
        }
        self.remount_case()?;
        Ok(RuntimeProgramUpdate {
            redraw: RuntimeRedraw::Window(id),
            window_commands: vec![WindowCommand::SetTitle {
                id,
                title: self.window_title(),
            }],
            exit: false,
        })
    }
}

impl App {
    fn window_title(&self) -> String {
        let theme = match self.theme {
            ThemeMode::Dark => "dark",
            ThemeMode::Light => "light",
        };
        format!(
            "NanaUI Runtime SceneWgpuPainter — {} ({theme})",
            self.case.label()
        )
    }

    fn remount_case(&mut self) -> Result<(), FrameworkError> {
        self.document = RuntimeDocument::new(DocumentId::new(1).expect("document"));
        remount(
            &mut self.document,
            self.case,
            self.theme,
            &self.graph,
            self.graph_viewport,
            self.graph_selection.as_ref(),
        )
    }
}

fn remount(
    document: &mut RuntimeDocument,
    case: Case,
    theme: ThemeMode,
    graph: &GraphModel,
    graph_viewport: GraphViewport,
    graph_selection: Option<&GraphSelection>,
) -> Result<(), FrameworkError> {
    let document_id = document.document();
    document.context_mut().set_theme(theme)?;
    match case {
        Case::GraphCanvas => {
            document.context_mut().build(document_id, |ui| {
                ui.child(
                    "graph",
                    RuntimeGraphCanvas::new("ab", graph.clone())
                        .viewport(graph_viewport)
                        .selection(graph_selection.cloned()),
                );
            })?;
        }
        Case::Workspace => {
            let workspace = document.context_mut().build(document_id, |ui| {
                let nav = ui.leaf(runtime_slot_text("Nav"));
                let files = ui.leaf(runtime_slot_text("Files"));
                let toolbar = ui.leaf(runtime_slot_text("Toolbar"));
                let primary = ui.leaf(runtime_slot_text("Primary"));
                let inspector = ui.leaf(runtime_slot_text("Inspector"));
                let diagnostics = ui.leaf(runtime_slot_text("Diagnostics"));
                let workspace = ui.child(
                    "workspace",
                    Workspace::from_model(
                        &WorkspaceModel::new(),
                        [
                            WorkspaceRegionSlot::new(RegionId::GlobalNavigation, nav.stable_id()),
                            WorkspaceRegionSlot::new(RegionId::Resources, files.stable_id()),
                            WorkspaceRegionSlot::new(RegionId::PrimaryToolbar, toolbar.stable_id()),
                            WorkspaceRegionSlot::new(RegionId::Primary, primary.stable_id()),
                            WorkspaceRegionSlot::new(RegionId::Inspector, inspector.stable_id()),
                            WorkspaceRegionSlot::new(
                                RegionId::Diagnostics,
                                diagnostics.stable_id(),
                            ),
                        ],
                    ),
                );
                ui.nest(workspace, |ui| {
                    ui.adopt(nav);
                    ui.adopt(files);
                    ui.adopt(toolbar);
                    ui.adopt(primary);
                    ui.adopt(inspector);
                    ui.adopt(diagnostics);
                });
                workspace
            })?;
            document.context_mut().assemble_workspace(workspace)?;
        }
        Case::Dock => {
            let dock = document.context_mut().build(document_id, |ui| {
                let nav = ui.leaf(runtime_slot_text("Nav"));
                let files = ui.leaf(runtime_slot_text("Files"));
                let primary = ui.leaf(runtime_slot_text("Primary"));
                let dock = ui.child(
                    "dock",
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
                );
                ui.nest(dock, |ui| {
                    ui.adopt(nav);
                    ui.adopt(files);
                    ui.adopt(primary);
                });
                dock
            })?;
            document.context_mut().assemble_dock(dock)?;
        }
        Case::DockPanel => {
            document.context_mut().build(document_id, |ui| {
                let body = ui.leaf(RuntimeText::new("Inspector"));
                let panel = ui.child(
                    "panel",
                    DockPanel::new().padding(10.0).content(body.stable_id()),
                );
                ui.nest(panel, |ui| ui.adopt(body));
            })?;
        }
        Case::SplitPane => {
            let pane = document.context_mut().build(document_id, |ui| {
                let first = ui.leaf(runtime_slot_text("First"));
                let second = ui.leaf(runtime_slot_text("Second"));
                let pane = ui.child(
                    "pane",
                    SplitPane::from_model(
                        &SplitPaneModel::new(SplitAxis::Horizontal, 180.0, 80.0, 320.0),
                        first.stable_id(),
                        second.stable_id(),
                    ),
                );
                ui.nest(pane, |ui| {
                    ui.adopt(first);
                    ui.adopt(second);
                });
                pane
            })?;
            document.context_mut().assemble_split_pane(pane)?;
        }
        Case::PaneChrome => {
            document.context_mut().build(document_id, |ui| {
                let header = ui.leaf(RuntimeText::new(""));
                let tabs = ui.leaf(RuntimeText::new("editor.rs"));
                let body = ui.leaf(RuntimeText::new("Body"));
                let close = ui.leaf(RuntimeText::new("关闭"));
                ui.nest(header, |ui| {
                    ui.adopt(tabs);
                    ui.adopt(close);
                });
                let chrome = ui.child(
                    "chrome",
                    PaneChrome::new()
                        .header(header.stable_id())
                        .tabs(tabs.stable_id())
                        .body(body.stable_id())
                        .actions([
                            PaneChromeAction::new(PaneChromeActionKind::CloseItem, "关闭")
                                .target(close.stable_id()),
                        ]),
                );
                ui.nest(chrome, |ui| {
                    ui.adopt(header);
                    ui.adopt(body);
                });
            })?;
        }
        Case::PaneTree => {
            document.context_mut().build(document_id, |ui| {
                let left = ui.leaf(runtime_slot_text("left"));
                let right = ui.leaf(runtime_slot_text("right"));
                let tree = ui.child(
                    "tree",
                    PaneTree::new(PaneTreeNode::split(
                        "root",
                        SplitAxis::Horizontal,
                        0.4,
                        PaneTreeNode::leaf_content("left", left.stable_id()),
                        PaneTreeNode::leaf_content("right", right.stable_id()),
                    )),
                );
                ui.nest(tree, |ui| {
                    ui.adopt(left);
                    ui.adopt(right);
                });
            })?;
        }
        Case::AppShell => {
            let shell = document.context_mut().build(document_id, |ui| {
                let title = ui.leaf(AppTitleBar::new("NanaUI"));
                let body = ui.leaf(RuntimeText::new("Workspace"));
                let shell = ui.child(
                    "shell",
                    AppShell::new()
                        .title_bar(title.stable_id())
                        .body(body.stable_id()),
                );
                ui.nest(shell, |ui| {
                    ui.adopt(title);
                    ui.adopt(body);
                });
                shell
            })?;
            document.context_mut().assemble_app_shell(shell)?;
        }
        Case::AppTitleBar => {
            document.context_mut().build(document_id, |ui| {
                ui.child("title", AppTitleBar::new("NanaUI"));
            })?;
        }
        Case::SettingsPage => {
            let (model, state) = ab_settings();
            let page = document.context_mut().build(document_id, |ui| {
                let content = ui.leaf(RuntimeText::new("Appearance content"));
                let page = ui.child(
                    "page",
                    SettingsPage::new(model.clone(), state.clone()).content(content.stable_id()),
                );
                ui.nest(page, |ui| ui.adopt(content));
                page
            })?;
            document.context_mut().assemble_settings_page(page)?;
        }
        Case::Calendar => {
            document.context_mut().build(document_id, |ui| {
                ui.child(
                    "calendar",
                    RuntimeCalendarHeatmap::new(ab_calendar_data()).label("活动"),
                );
            })?;
        }
    }
    Ok(())
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

fn ab_graph() -> GraphModel {
    let source = GraphNode::new(
        "source",
        "Source",
        GraphPoint::new(24.0, 88.0),
        GraphSize::new(140.0, 80.0),
    )
    .with_port(GraphPort::new(
        "out",
        "Out",
        GraphPortKind::Output,
        GraphPortSide::Right,
    ));
    let transform = GraphNode::new(
        "transform",
        "Transform",
        GraphPoint::new(220.0, 48.0),
        GraphSize::new(168.0, 128.0),
    )
    .with_port(GraphPort::new(
        "in",
        "In",
        GraphPortKind::Input,
        GraphPortSide::Left,
    ))
    .with_port(GraphPort::new(
        "out",
        "Out",
        GraphPortKind::Output,
        GraphPortSide::Right,
    ));
    let target = GraphNode::new(
        "target",
        "Target",
        GraphPoint::new(444.0, 88.0),
        GraphSize::new(140.0, 80.0),
    )
    .with_port(GraphPort::new(
        "in",
        "In",
        GraphPortKind::Input,
        GraphPortSide::Left,
    ));
    GraphModel::new(
        vec![source, transform, target],
        vec![
            GraphEdge::new(
                "source-transform",
                GraphEndpoint::new("source", "out"),
                GraphEndpoint::new("transform", "in"),
            ),
            GraphEdge::new(
                "transform-target",
                GraphEndpoint::new("transform", "out"),
                GraphEndpoint::new("target", "in"),
            ),
        ],
    )
    .expect("A/B graph is valid")
}

fn ab_settings() -> (&'static SettingsModel, &'static SettingsState) {
    static MODEL: std::sync::OnceLock<SettingsModel> = std::sync::OnceLock::new();
    static STATE: std::sync::OnceLock<SettingsState> = std::sync::OnceLock::new();
    let model = MODEL.get_or_init(|| {
        SettingsModel::new(
            "appearance",
            [
                SettingsTab::new("appearance", "外观"),
                SettingsTab::new("about", "关于").full_page(true),
            ],
        )
        .expect("A/B settings model")
    });
    let state = STATE.get_or_init(|| SettingsState::new(model));
    (model, state)
}

fn ab_calendar_data() -> [RuntimeCalendarHeatmapDatum; 3] {
    [
        RuntimeCalendarHeatmapDatum::new("2026-06-01", 1.0),
        RuntimeCalendarHeatmapDatum::new("2026-06-02", 4.0),
        RuntimeCalendarHeatmapDatum::new("2026-06-03", 8.0),
    ]
}
