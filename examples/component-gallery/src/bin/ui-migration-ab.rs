//! Windowed Iced | Runtime A/B for migration review.
//!
//! Left = Iced compatibility composer. Right = Runtime Scene via IcedSceneView.
//! Keys: Left/Right or 1-9 select a component, T toggles theme.
//! GPU slots stay out of this window: they need the host Device/Queue, and
//! this binary must not create a second wgpu context.

use iced::keyboard::{self, Key};
use iced::widget::{column, container, row, text};
use iced::{Element, Event, Length, Point, Size, Subscription};
use nana_ui::RuntimeInputAdapter;
use nana_ui::compatibility::{
    AppTitleBar as IcedAppTitleBar, CalendarHeatmap as IcedCalendarHeatmap,
    CalendarHeatmapDatum as IcedCalendarHeatmapDatum,
    CalendarHeatmapModel as IcedCalendarHeatmapModel,
    CalendarHeatmapOptions as IcedCalendarHeatmapOptions, DockPanel as IcedDockPanel,
    GraphCanvas as IcedGraphCanvas, PaneChrome as IcedPaneChrome,
    PaneChromeAction as IcedPaneChromeAction, PaneChromeActionKind as IcedPaneChromeActionKind,
    PaneTree as IcedPaneTree, PaneTreeNode as IcedPaneTreeNode, build_calendar_heatmap_model,
};
use nana_ui::runtime::{
    AppShell, AppTitleBar, CalendarHeatmap as RuntimeCalendarHeatmap,
    CalendarHeatmapDatum as RuntimeCalendarHeatmapDatum, Dock, DockAxis, DockNode, DockPanel,
    DocumentId, Entity, GraphCanvas as RuntimeGraphCanvas, GraphInteraction, LayoutViewport,
    PaneChrome, PaneChromeAction, PaneChromeActionKind, PaneTree, PaneTreeNode, SettingsPage,
    SplitPane, Text as RuntimeText, Workspace, WorkspaceRegionSlot,
};
use nana_ui::{
    DockContents, DockController, DockId, DockItemSpec, DockLayout, DockNode as IcedDockNode,
    DockSurfaceId, GraphEdge, GraphEndpoint, GraphModel, GraphNode, GraphPoint, GraphPort,
    GraphPortKind, GraphPortSide, GraphSize, GraphViewport, IcedSceneView, IcedTextShaper,
    RegionId, SettingsModel, SettingsState, SettingsTab, SplitAxis, SplitPaneController, ThemeMode,
    ThemeModeExt, ThemeTokens, WorkspaceController, WorkspaceSlots, app_shell, dock_workspace,
    ratio_pane_split, settings_page, split_pane, ui_font, ui_font_defaults, ui_font_sources,
    workspace_view,
};
use nana_ui::{GraphCanvasEvent, GraphSelection, SplitPaneAction};
use nana_ui_core::{SplitPaneModel, WorkspaceModel};
use nana_ui_platform::{InputEvent, InputModifiers, PointerPhase, PointerType};
use nana_ui_scene::RuntimeDocument;
use std::sync::{Arc, Mutex};

const PANEL: Size = Size::new(560.0, 360.0);
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

#[derive(Clone, Debug)]
enum Message {
    Next,
    Prev,
    Theme,
    Idle,
    Select(usize),
    IcedGraph(GraphCanvasEvent),
    IcedSplit(SplitPaneAction),
    RuntimeMove(Point),
    RuntimeDown { button: i16, point: Point },
    RuntimeUp { button: i16, point: Point },
    RuntimeScroll(iced::mouse::ScrollDelta),
    RuntimeKey(String),
}

struct App {
    theme: ThemeMode,
    case: Case,
    graph: GraphModel,
    graph_viewport: GraphViewport,
    graph_selection: Option<GraphSelection>,
    split: SplitPaneController,
    calendar_model: IcedCalendarHeatmapModel<()>,
    document: RuntimeDocument,
    canvas: Option<nana_ui::runtime::StableNodeId>,
    graph_events: Arc<Mutex<Vec<GraphCanvasEvent>>>,
    last_runtime_pointer: Point,
    runtime_focused: bool,
}

fn main() -> iced::Result {
    let mut application =
        iced::application(|| (App::new(), ui_font_defaults()), App::update, App::view)
            .title("NanaUI Iced | Runtime A/B")
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
        let graph = ab_graph();
        let graph_viewport = graph
            .bounds()
            .map(|bounds| {
                GraphViewport::fit(bounds, GraphSize::new(PANEL.width, PANEL.height), 28.0)
            })
            .unwrap_or_default();
        let mut app = Self {
            theme: ThemeMode::Dark,
            case: Case::GraphCanvas,
            graph,
            graph_viewport,
            graph_selection: None,
            split: SplitPaneController::new(SplitAxis::Horizontal, 180.0, 80.0, 320.0),
            calendar_model: ab_calendar_model(),
            document: RuntimeDocument::new(DocumentId::new(1).expect("document")),
            canvas: None,
            graph_events: Arc::new(Mutex::new(Vec::new())),
            last_runtime_pointer: Point::ORIGIN,
            runtime_focused: true,
        };
        app.remount_case();
        app
    }

    fn remount_case(&mut self) {
        self.document = RuntimeDocument::new(DocumentId::new(1).expect("document"));
        self.canvas = remount(
            &mut self.document,
            self.case,
            self.theme,
            &self.graph,
            self.graph_viewport,
            self.graph_selection.as_ref(),
            &self.split,
            &self.graph_events,
        )
        .ok()
        .flatten();
    }

    fn apply_graph_event(&mut self, event: GraphCanvasEvent) {
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

    fn sync_runtime_graph(&mut self) {
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
        let _ = self.document.flush(
            LayoutViewport::new(PANEL.width, PANEL.height),
            &mut IcedTextShaper,
        );
    }

    fn dispatch_runtime(&mut self, event: InputEvent) {
        let document = self.document.document();
        let _ =
            RuntimeInputAdapter::default().dispatch(self.document.context_mut(), document, &event);
        let events = std::mem::take(&mut *self.graph_events.lock().expect("graph events"));
        for event in events {
            self.apply_graph_event(event);
        }
        if self.canvas.is_some() {
            self.sync_runtime_graph();
        } else {
            let _ = self.document.flush(
                LayoutViewport::new(PANEL.width, PANEL.height),
                &mut IcedTextShaper,
            );
        }
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Next => {
                self.case = self.case.next();
                self.remount_case();
            }
            Message::Prev => {
                self.case = self.case.prev();
                self.remount_case();
            }
            Message::Idle => {}
            Message::Theme => {
                self.theme = match self.theme {
                    ThemeMode::Dark => ThemeMode::Light,
                    ThemeMode::Light => ThemeMode::Dark,
                };
                self.remount_case();
            }
            Message::Select(index) => {
                if let Some(case) = Case::ALL.get(index) {
                    self.case = *case;
                    self.remount_case();
                }
            }
            Message::IcedGraph(event) => {
                self.runtime_focused = false;
                self.apply_graph_event(event);
                self.sync_runtime_graph();
            }
            Message::IcedSplit(action) => {
                self.runtime_focused = false;
                let _ = self.split.update(action);
            }
            Message::RuntimeMove(point) => {
                self.runtime_focused = true;
                self.last_runtime_pointer = point;
                self.dispatch_runtime(runtime_pointer(PointerPhase::Move, point, 0));
            }
            Message::RuntimeDown { button, point } => {
                self.runtime_focused = true;
                self.last_runtime_pointer = point;
                self.dispatch_runtime(runtime_pointer(PointerPhase::Down, point, button));
            }
            Message::RuntimeUp { button, point } => {
                self.runtime_focused = true;
                self.last_runtime_pointer = point;
                self.dispatch_runtime(runtime_pointer(PointerPhase::Up, point, button));
            }
            Message::RuntimeScroll(delta) => {
                let (delta_y, line_delta) = match delta {
                    iced::mouse::ScrollDelta::Lines { y, .. } => (y, true),
                    iced::mouse::ScrollDelta::Pixels { y, .. } => (y, false),
                };
                self.dispatch_runtime(InputEvent::Wheel {
                    x: self.last_runtime_pointer.x,
                    y: self.last_runtime_pointer.y,
                    delta_x: 0.0,
                    delta_y,
                    line_delta,
                    modifiers: InputModifiers::default(),
                });
            }
            Message::RuntimeKey(key) => {
                if matches!(self.case, Case::GraphCanvas | Case::SplitPane | Case::Dock)
                    && self.runtime_focused
                {
                    self.dispatch_runtime(InputEvent::Keyboard {
                        pressed: true,
                        key: key.clone(),
                        text: None,
                        code: key,
                        repeat: false,
                        modifiers: InputModifiers::default(),
                    });
                } else if key == "ArrowRight" {
                    self.case = self.case.next();
                    self.remount_case();
                } else if key == "ArrowLeft" {
                    self.case = self.case.prev();
                    self.remount_case();
                }
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        iced::event::listen_with(|event, _status, _id| match event {
            Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => match key {
                Key::Named(keyboard::key::Named::ArrowRight) => {
                    Some(Message::RuntimeKey("ArrowRight".into()))
                }
                Key::Named(keyboard::key::Named::ArrowLeft) => {
                    Some(Message::RuntimeKey("ArrowLeft".into()))
                }
                Key::Named(keyboard::key::Named::ArrowUp) => {
                    Some(Message::RuntimeKey("ArrowUp".into()))
                }
                Key::Named(keyboard::key::Named::ArrowDown) => {
                    Some(Message::RuntimeKey("ArrowDown".into()))
                }
                Key::Named(keyboard::key::Named::Home) => Some(Message::RuntimeKey("Home".into())),
                Key::Named(keyboard::key::Named::Escape) => {
                    Some(Message::RuntimeKey("Escape".into()))
                }
                Key::Character(ch) if ch.eq_ignore_ascii_case("t") => Some(Message::Theme),
                Key::Character(ch) if ch == "[" => Some(Message::Prev),
                Key::Character(ch) if ch == "]" => Some(Message::Next),
                Key::Character(ch) if matches!(ch.as_str(), "+" | "=" | "-" | "0") => {
                    Some(Message::RuntimeKey(ch.to_string()))
                }
                Key::Character(ch) => ch.parse::<usize>().ok().and_then(|n| {
                    (1..=9)
                        .contains(&n)
                        .then_some(Message::Select(n.saturating_sub(1)))
                }),
                _ => None,
            },
            _ => None,
        })
    }

    fn runtime_cursor(&self) -> iced::mouse::Interaction {
        let point = self.last_runtime_pointer;
        let document = self.document.document();
        let context = self.document.context();
        if let Some(captured) = context.world().pointer_capture(document, 1) {
            if let Some(axis) = context.split_handle_axis(captured) {
                return split_cursor(axis);
            }
            if context.is_graph_canvas(captured) {
                let entity = Entity::<RuntimeGraphCanvas>::from_stable_id(captured);
                return context
                    .read(entity, |canvas| match canvas.interaction {
                        GraphInteraction::Pan { .. } => iced::mouse::Interaction::Grabbing,
                        GraphInteraction::NodeDrag { .. } | GraphInteraction::Connection { .. } => {
                            iced::mouse::Interaction::Pointer
                        }
                        GraphInteraction::None => iced::mouse::Interaction::Grab,
                    })
                    .unwrap_or(iced::mouse::Interaction::Grab);
            }
        }
        let Some(target) = context.pointer_target(document, point.x, point.y) else {
            return iced::mouse::Interaction::None;
        };
        if let Some(handle) = context.split_handle_near(document, point.x, point.y)
            && let Some(axis) = context.split_handle_axis(handle)
        {
            return split_cursor(axis);
        }
        if context.is_calendar_heatmap(target) {
            return iced::mouse::Interaction::Crosshair;
        }
        if context.is_graph_canvas(target) {
            let entity = Entity::<RuntimeGraphCanvas>::from_stable_id(target);
            let local = context
                .world()
                .layout_box(target)
                .map(|bounds| GraphPoint::new(point.x - bounds.x, point.y - bounds.y));
            let hit = local.and_then(|local| {
                context
                    .read(entity, |canvas| canvas.hit_test(local))
                    .ok()
                    .flatten()
            });
            return if hit.is_some() {
                iced::mouse::Interaction::Pointer
            } else {
                iced::mouse::Interaction::Grab
            };
        }
        iced::mouse::Interaction::None
    }

    fn view(&self) -> Element<'_, Message> {
        let tokens = self.theme.tokens();
        let colors = tokens.colors;
        let iced_side = panel(
            "Iced",
            colors,
            iced_case(
                self.case,
                tokens,
                &self.graph,
                self.graph_viewport,
                self.graph_selection.as_ref(),
                &self.split,
                &self.calendar_model,
            ),
        );
        let runtime_view: Element<'_, Message> = IcedSceneView::new(self.document.scene(), PANEL)
            .map_or_else(
                |error| {
                    text(format!("Runtime scene failed: {error}"))
                        .size(12)
                        .into()
                },
                Into::into,
            );
        let runtime_side = panel(
            "Runtime",
            colors,
            scene_pointer(runtime_view, self.runtime_cursor()),
        );
        column![
            text(format!(
                "{}   [/] or 1-9 switch   T theme ({})   dock/split/calendar: interact on Runtime",
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

fn iced_case<'a>(
    case: Case,
    tokens: ThemeTokens,
    graph: &'a GraphModel,
    graph_viewport: GraphViewport,
    graph_selection: Option<&'a GraphSelection>,
    split: &'a SplitPaneController,
    calendar: &'a IcedCalendarHeatmapModel<()>,
) -> Element<'a, Message> {
    match case {
        Case::GraphCanvas => IcedGraphCanvas::new(
            "ab",
            graph,
            graph_viewport,
            graph_selection,
            Message::IcedGraph,
            tokens,
        )
        .view(),
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
        Case::SplitPane => split_pane(
            split,
            slot_label("First", tokens),
            slot_label("Second", tokens),
            Message::IcedSplit,
            tokens,
        ),
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
        Case::SettingsPage => {
            let (model, state) = ab_settings();
            settings_page(
                model,
                state,
                text("Appearance content")
                    .size(13)
                    .color(tokens.colors.text),
                tokens,
            )
        }
        Case::Calendar => IcedCalendarHeatmap::new(calendar, |_| Message::Idle, tokens).view(),
    }
}

fn remount(
    document: &mut RuntimeDocument,
    case: Case,
    theme: ThemeMode,
    graph: &GraphModel,
    graph_viewport: GraphViewport,
    graph_selection: Option<&GraphSelection>,
    split: &SplitPaneController,
    graph_events: &Arc<Mutex<Vec<GraphCanvasEvent>>>,
) -> Result<Option<nana_ui::runtime::StableNodeId>, Box<dyn std::error::Error>> {
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
        Case::GraphCanvas => {
            let canvas = document.context_mut().create_component(
                document_id,
                RuntimeGraphCanvas::new("ab", graph.clone())
                    .viewport(graph_viewport)
                    .selection(graph_selection.cloned()),
            )?;
            let observed = Arc::clone(graph_events);
            document
                .context_mut()
                .on(canvas, move |_canvas, event: &GraphCanvasEvent, _cx| {
                    observed.lock().expect("graph events").push(event.clone());
                })?;
            canvas.stable_id()
        }
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
            let pane = document.context_mut().create_component(
                document_id,
                SplitPane::from_model(
                    &SplitPaneModel::new(SplitAxis::Horizontal, 180.0, 80.0, 320.0),
                    first.stable_id(),
                    second.stable_id(),
                ),
            )?;
            document.context_mut().append_child(pane, first)?;
            document.context_mut().append_child(pane, second)?;
            document.context_mut().assemble_split_pane(pane)?;
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
            document.context_mut().assemble_app_shell(shell)?;
            shell.stable_id()
        }
        Case::AppTitleBar => document
            .context_mut()
            .create_component(document_id, AppTitleBar::new("NanaUI"))?
            .stable_id(),
        Case::SettingsPage => {
            let content = bare(document, "Appearance content")?;
            let (model, state) = ab_settings();
            let page = document.context_mut().create_component(
                document_id,
                SettingsPage::new(model.clone(), state.clone()).content(content.stable_id()),
            )?;
            document.context_mut().append_child(page, content)?;
            document.context_mut().assemble_settings_page(page)?;
            page.stable_id()
        }
        Case::Calendar => document
            .context_mut()
            .create_component(
                document_id,
                RuntimeCalendarHeatmap::new(ab_calendar_data()).label("活动"),
            )?
            .stable_id(),
    };
    let _ = split;
    let canvas = matches!(case, Case::GraphCanvas).then_some(target);
    let _ = document.flush(
        LayoutViewport::new(PANEL.width, PANEL.height),
        &mut IcedTextShaper,
    )?;
    Ok(canvas)
}

fn runtime_pointer(phase: PointerPhase, point: Point, button: i16) -> InputEvent {
    InputEvent::Pointer {
        phase,
        pointer_id: 1,
        pointer_type: PointerType::Mouse,
        x: point.x,
        y: point.y,
        screen_x: point.x,
        screen_y: point.y,
        button,
        buttons: if matches!(phase, PointerPhase::Down | PointerPhase::Move) && button == 0 {
            1
        } else {
            0
        },
        pressure: 0.5,
        tangential_pressure: 0.0,
        tilt_x: 0,
        tilt_y: 0,
        twist: 0,
        is_primary: button == 0,
        modifiers: InputModifiers::default(),
    }
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

fn ab_calendar_model() -> IcedCalendarHeatmapModel<()> {
    build_calendar_heatmap_model(
        &[
            IcedCalendarHeatmapDatum::new("2026-06-01", 1.0),
            IcedCalendarHeatmapDatum::new("2026-06-02", 4.0),
            IcedCalendarHeatmapDatum::new("2026-06-03", 8.0),
        ],
        IcedCalendarHeatmapOptions::default().week_starts_on(1),
    )
}

fn split_cursor(axis: SplitAxis) -> iced::mouse::Interaction {
    match axis {
        SplitAxis::Horizontal => iced::mouse::Interaction::ResizingHorizontally,
        SplitAxis::Vertical => iced::mouse::Interaction::ResizingVertically,
    }
}

fn scene_pointer(
    content: Element<'_, Message>,
    interaction: iced::mouse::Interaction,
) -> Element<'_, Message> {
    Element::new(ScenePointer {
        content,
        interaction,
    })
}

struct ScenePointer<'a> {
    content: Element<'a, Message>,
    interaction: iced::mouse::Interaction,
}

#[derive(Default)]
struct ScenePointerState {
    pressed: Option<i16>,
}

impl iced::advanced::Widget<Message, iced::Theme, iced::Renderer> for ScenePointer<'_> {
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::of::<ScenePointerState>()
    }

    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::new(ScenePointerState::default())
    }

    fn diff(&mut self, tree: &mut iced::advanced::widget::Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }

    fn size(&self) -> iced::Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut iced::advanced::widget::Tree,
        renderer: &iced::Renderer,
        limits: &iced::advanced::layout::Limits,
    ) -> iced::advanced::layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn update(
        &mut self,
        tree: &mut iced::advanced::widget::Tree,
        event: &iced::Event,
        layout: iced::advanced::Layout<'_>,
        cursor: iced::mouse::Cursor,
        renderer: &iced::Renderer,
        shell: &mut iced::advanced::Shell<'_, Message>,
        viewport: &iced::Rectangle,
    ) {
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            shell,
            viewport,
        );
        let state = tree.state.downcast_mut::<ScenePointerState>();
        let local = cursor.position_in(layout.bounds());
        match event {
            iced::Event::Mouse(iced::mouse::Event::CursorMoved { .. })
            | iced::Event::Touch(iced::touch::Event::FingerMoved { .. }) => {
                if let Some(point) = local.or_else(|| {
                    state
                        .pressed
                        .is_some()
                        .then(|| cursor.position())
                        .flatten()
                        .map(|position| {
                            Point::new(
                                position.x - layout.bounds().x,
                                position.y - layout.bounds().y,
                            )
                        })
                }) {
                    shell.publish(Message::RuntimeMove(point));
                }
            }
            iced::Event::Mouse(iced::mouse::Event::ButtonPressed(button)) => {
                let Some(point) = local else {
                    return;
                };
                let button = match button {
                    iced::mouse::Button::Left => 0,
                    iced::mouse::Button::Middle => 1,
                    _ => return,
                };
                state.pressed = Some(button);
                shell.publish(Message::RuntimeDown { button, point });
                shell.capture_event();
            }
            iced::Event::Mouse(iced::mouse::Event::ButtonReleased(button)) => {
                let button = match button {
                    iced::mouse::Button::Left => 0,
                    iced::mouse::Button::Middle => 1,
                    _ => return,
                };
                if state.pressed == Some(button) {
                    state.pressed = None;
                }
                let point = local.unwrap_or(Point::ORIGIN);
                shell.publish(Message::RuntimeUp { button, point });
                shell.capture_event();
            }
            iced::Event::Mouse(iced::mouse::Event::WheelScrolled { delta }) if local.is_some() => {
                shell.publish(Message::RuntimeScroll(*delta));
                shell.capture_event();
            }
            _ => {}
        }
    }

    fn draw(
        &self,
        tree: &iced::advanced::widget::Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &iced::advanced::renderer::Style,
        layout: iced::advanced::Layout<'_>,
        cursor: iced::mouse::Cursor,
        viewport: &iced::Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &iced::advanced::widget::Tree,
        layout: iced::advanced::Layout<'_>,
        cursor: iced::mouse::Cursor,
        viewport: &iced::Rectangle,
        renderer: &iced::Renderer,
    ) -> iced::mouse::Interaction {
        if cursor.is_over(layout.bounds()) && self.interaction != iced::mouse::Interaction::None {
            return self.interaction;
        }
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }
}
