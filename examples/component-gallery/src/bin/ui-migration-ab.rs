//! Windowed Runtime migration review painted by SceneWgpuPainter.
//!
//! Left/right Iced widget composers are gone. The Nana Scene host paints the
//! same Runtime fixtures used by snapshots. Keys: Left/Right or 1-9 select a
//! component, T toggles theme.

use std::sync::{Arc, Mutex};

use nana_ui::runtime::GraphCanvasEvent;
use nana_ui::runtime::view::{El, IntoView, entity_ref, widget};
use nana_ui::runtime::{
    AppShell, AppTitleBar, CalendarHeatmap as RuntimeCalendarHeatmap,
    CalendarHeatmapDatum as RuntimeCalendarHeatmapDatum, Dock, DockAxis, DockNode, DockPanel,
    DocumentId, Entity, FrameworkError, GraphCanvas as RuntimeGraphCanvas, PaneChrome,
    PaneChromeAction, PaneChromeActionKind, PaneTree, PaneTreeNode, SettingsPage, SplitPane,
    Text as RuntimeText, Workspace,
};
use nana_ui::{
    GraphEdge, GraphEndpoint, GraphModel, GraphNode, GraphPoint, GraphPort, GraphPortKind,
    GraphPortSide, GraphSelection, GraphSize, GraphViewport, RegionId, RoutedInput, RuntimeProgram,
    RuntimeProgramContext, RuntimeProgramUpdate, RuntimeRedraw, SettingsModel, SettingsState,
    SettingsTab, SplitAxis, ThemeMode, WindowDescriptor, run_runtime_scene,
};
use nana_ui_core::{SplitPaneModel, WorkspaceModel};
use nana_ui_platform::host::WindowCommand;
use nana_ui_platform::{InputPayload, WindowId};
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
    canvas: Option<nana_ui::runtime::StableNodeId>,
    graph_events: Arc<Mutex<Vec<GraphCanvasEvent>>>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_runtime_scene::<App>(
        WindowDescriptor::new("NanaUI Runtime migration A/B")
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
            case: Case::GraphCanvas,
            graph,
            graph_viewport,
            graph_selection: None,
            document: RuntimeDocument::new(DocumentId::new(1).expect("document")),
            canvas: None,
            graph_events: Arc::new(Mutex::new(Vec::new())),
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
        input: RoutedInput<'_>,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        self.drain_graph_events();
        let InputPayload::Key(key) = &input.event.payload else {
            return Ok(RuntimeProgramUpdate::redraw(id));
        };
        if !key.is_pressed() {
            return Ok(RuntimeProgramUpdate::redraw(id));
        }
        let changed = match &*key.logical.0 {
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
            "NanaUI Runtime migration A/B — {} ({theme})",
            self.case.label()
        )
    }

    fn remount_case(&mut self) -> Result<(), FrameworkError> {
        self.document = RuntimeDocument::new(DocumentId::new(1).expect("document"));
        self.canvas = remount(
            &mut self.document,
            self.case,
            self.theme,
            &self.graph,
            self.graph_viewport,
            self.graph_selection.as_ref(),
            &self.graph_events,
        )?;
        Ok(())
    }

    fn drain_graph_events(&mut self) {
        let events = std::mem::take(&mut *self.graph_events.lock().expect("graph events"));
        for event in events {
            match event {
                GraphCanvasEvent::SelectionChanged(selection) => self.graph_selection = selection,
                GraphCanvasEvent::ViewportInput(viewport)
                | GraphCanvasEvent::ViewportChanged(viewport) => self.graph_viewport = viewport,
                GraphCanvasEvent::NodePositionInput { node, position }
                | GraphCanvasEvent::NodePositionChanged { node, position } => {
                    let _ = self.graph.set_node_position(&node, position);
                }
                GraphCanvasEvent::ConnectionRequested { source, target } => {
                    let edge_id = format!("ab-edge-{}", self.graph.edges().len() + 1);
                    let _ = self.graph.add_edge(GraphEdge::new(edge_id, source, target));
                }
            }
        }
        let Some(canvas) = self.canvas else {
            return;
        };
        let entity = Entity::<RuntimeGraphCanvas>::from_stable_id(canvas);
        let graph = self.graph.clone();
        let viewport = self.graph_viewport;
        let selection = self.graph_selection.clone();
        let _ = self
            .document
            .context_mut()
            .update_component(entity, |canvas, _| {
                canvas.set_model(graph);
                canvas.set_viewport(viewport);
                canvas.set_selection(selection);
            });
    }
}

fn remount(
    document: &mut RuntimeDocument,
    case: Case,
    theme: ThemeMode,
    graph: &GraphModel,
    graph_viewport: GraphViewport,
    graph_selection: Option<&GraphSelection>,
    graph_events: &Arc<Mutex<Vec<GraphCanvasEvent>>>,
) -> Result<Option<nana_ui::runtime::StableNodeId>, FrameworkError> {
    let document_id = document.document();
    document.context_mut().set_theme(theme)?;
    let target = match case {
        Case::GraphCanvas => {
            let observed = Arc::clone(graph_events);
            mount(document, || {
                widget(
                    RuntimeGraphCanvas::new("ab", graph.clone())
                        .viewport(graph_viewport)
                        .selection(graph_selection.cloned()),
                )
                .on(move |event: &GraphCanvasEvent| {
                    observed.lock().expect("graph events").push(event.clone());
                })
            })?
        }
        Case::Workspace => mount(document, || {
            widget(Workspace::from_model(&WorkspaceModel::new(), []))
                .region(RegionId::GlobalNavigation, slot_text("Nav"))
                .region(RegionId::Resources, slot_text("Files"))
                .region(RegionId::PrimaryToolbar, slot_text("Toolbar"))
                .region(RegionId::Primary, slot_text("Primary"))
                .region(RegionId::Inspector, slot_text("Inspector"))
                .region(RegionId::Diagnostics, slot_text("Diagnostics"))
        })?,
        Case::Dock => mount(document, || {
            // Panels are children keyed with their item id; the dock binds
            // each to its item when it assembles.
            widget(
                Dock::new(DockNode::split(
                    DockAxis::Horizontal,
                    0.35,
                    DockNode::tabs(["nav", "files"], "nav", [("nav", None), ("files", None)]),
                    DockNode::item("primary", None),
                ))
                .title("nav", "Nav")
                .title("files", "Files")
                .title("primary", "Primary"),
            )
            .children((
                slot_text("Nav").key("nav"),
                slot_text("Files").key("files"),
                slot_text("Primary").key("primary"),
            ))
        })?,
        Case::DockPanel => mount(document, || {
            widget(DockPanel::new().padding(10.0))
                .child_slot(widget(RuntimeText::new("Inspector")), DockPanel::content)
        })?,
        Case::SplitPane => mount(document, || {
            widget(SplitPane::new(&SplitPaneModel::new(
                SplitAxis::Horizontal,
                180.0,
                80.0,
                320.0,
            )))
            .first(slot_text("First"))
            .second(slot_text("Second"))
        })?,
        Case::PaneChrome => mount(document, || {
            let (tabs, close) = (entity_ref::<RuntimeText>(), entity_ref::<RuntimeText>());
            let header = widget(RuntimeText::new("")).children((
                widget(RuntimeText::new("editor.rs")).entity_ref(tabs),
                widget(RuntimeText::new("关闭")).entity_ref(close),
            ));
            let built = "the header is built before the chrome";
            widget(PaneChrome::new())
                .child_slot(header, move |chrome, header| {
                    chrome
                        .header(header)
                        .tabs(tabs.get().expect(built).stable_id())
                        .actions([
                            PaneChromeAction::new(PaneChromeActionKind::CloseItem, "关闭")
                                .target(close.get().expect(built).stable_id()),
                        ])
                })
                .child_slot(widget(RuntimeText::new("Body")), PaneChrome::body)
        })?,
        Case::PaneTree => mount(document, || {
            widget(PaneTree::new(PaneTreeNode::split(
                "root",
                SplitAxis::Horizontal,
                0.4,
                PaneTreeNode::leaf("left"),
                PaneTreeNode::leaf("right"),
            )))
            .child_slot(slot_text("left"), |tree, id| bind_pane(tree, true, id))
            .child_slot(slot_text("right"), |tree, id| bind_pane(tree, false, id))
        })?,
        Case::AppShell => mount(document, || {
            widget(AppShell::new())
                .title_bar(widget(AppTitleBar::new("NanaUI")))
                .body(widget(RuntimeText::new("Workspace")))
        })?,
        // Not a view: a view-built title bar assembles itself, and this case
        // shows the bar as a bare component, like the snapshot fixture.
        Case::AppTitleBar => document
            .context_mut()
            .create_component(document_id, AppTitleBar::new("NanaUI"))?
            .stable_id(),
        Case::SettingsPage => {
            let (model, state) = ab_settings();
            mount(document, || {
                widget(SettingsPage::new(model.clone(), state.clone()))
                    .content(widget(RuntimeText::new("Appearance content")))
            })?
        }
        Case::Calendar => mount(document, || {
            widget(RuntimeCalendarHeatmap::new(ab_calendar_data()).label("活动"))
        })?,
    };
    Ok(Some(target))
}

/// Mount `view` as the document's root; its node.
fn mount<V: IntoView>(
    document: &mut RuntimeDocument,
    view: impl FnOnce() -> V,
) -> Result<nana_ui::runtime::StableNodeId, FrameworkError> {
    let document_id = document.document();
    let mounted = document.context_mut().mount_view_root(document_id, view)?;
    Ok(mounted.roots()[0])
}

fn slot_text(value: &str) -> El<RuntimeText> {
    widget(runtime_slot_text(value))
}

/// `tree`, a split of two leaves, with `content` in its first leaf or its
/// second.
fn bind_pane(mut tree: PaneTree, first: bool, content: nana_ui::runtime::StableNodeId) -> PaneTree {
    if let PaneTreeNode::Split {
        first: leaf_one,
        second: leaf_two,
        ..
    } = &mut tree.root
        && let PaneTreeNode::Leaf { content: slot, .. } = if first {
            &mut **leaf_one
        } else {
            &mut **leaf_two
        }
    {
        *slot = Some(content);
    }
    tree
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
