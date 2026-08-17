//! Windowed IcedSceneView | SceneWgpuPainter A/B.
//!
//! Left = current Runtime paint through IcedSceneView.
//! Right = the same UiScene through SceneWgpuPainter.
//! Keys: Left/Right or 1-9 select a component, T toggles theme.

use std::sync::{Arc, Mutex};

use iced::keyboard::{self, Key};
use iced::widget::shader::{self, Viewport};
use iced::widget::{column, container, mouse_area, row, space, text};
use iced::{Element, Event, Length, Point, Rectangle, Size, Subscription, wgpu};
use nana_ui::runtime::{
    AppShell, AppTitleBar, CalendarHeatmap as RuntimeCalendarHeatmap,
    CalendarHeatmapDatum as RuntimeCalendarHeatmapDatum, Dock, DockAxis, DockNode, DockPanel,
    DocumentId, GraphCanvas as RuntimeGraphCanvas, LayoutViewport, PaneChrome, PaneChromeAction,
    PaneChromeActionKind, PaneTree, PaneTreeNode, SettingsPage, SplitPane, Text as RuntimeText,
    Workspace, WorkspaceRegionSlot,
};
use nana_ui::{
    GraphEdge, GraphEndpoint, GraphModel, GraphNode, GraphPoint, GraphPort, GraphPortKind,
    GraphPortSide, GraphSelection, GraphSize, GraphViewport, IcedSceneView, IcedTextShaper,
    RegionId, RuntimeInputAdapter, SceneGpuRendererRegistry, ScenePaintViewport, SceneWgpuPainter,
    SettingsModel, SettingsState, SettingsTab, SplitAxis, ThemeMode, ThemeModeExt,
    default_scene_gpu_renderers_with_host, ui_font, ui_font_defaults, ui_font_sources,
};
use nana_ui_core::{SplitPaneModel, WorkspaceModel};
use nana_ui_platform::{InputEvent, InputModifiers, PointerPhase, PointerType};
use nana_ui_scene::{RuntimeDocument, UiScene};

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
    Select(usize),
    Reflow,
    PointerMove(Point),
    PointerDown,
    PointerUp,
    PointerScroll(iced::mouse::ScrollDelta),
}

struct App {
    theme: ThemeMode,
    case: Case,
    graph: GraphModel,
    graph_viewport: GraphViewport,
    graph_selection: Option<GraphSelection>,
    document: RuntimeDocument,
    last_pointer: Point,
}

fn main() -> iced::Result {
    let mut application =
        iced::application(|| (App::new(), ui_font_defaults()), App::update, App::view)
            .title("IcedSceneView | SceneWgpuPainter")
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
            case: Case::SettingsPage,
            graph,
            graph_viewport,
            graph_selection: None,
            document: RuntimeDocument::new(DocumentId::new(1).expect("document")),
            last_pointer: Point::ORIGIN,
        };
        app.remount_case();
        app
    }

    fn remount_case(&mut self) {
        self.document = RuntimeDocument::new(DocumentId::new(1).expect("document"));
        let _ = remount(
            &mut self.document,
            self.case,
            self.theme,
            &self.graph,
            self.graph_viewport,
            self.graph_selection.as_ref(),
        );
        self.flush_scene();
    }

    fn flush_scene(&mut self) {
        let _ = self.document.flush(
            LayoutViewport::new(PANEL.width, PANEL.height),
            &mut IcedTextShaper,
        );
    }

    fn dispatch_pointer(&mut self, phase: PointerPhase, button: i16) {
        let point = self.last_pointer;
        let document = self.document.document();
        let _ = RuntimeInputAdapter::default().dispatch(
            self.document.context_mut(),
            document,
            &runtime_pointer(phase, point, button),
        );
        self.flush_scene();
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
            Message::Reflow => self.flush_scene(),
            Message::PointerMove(point) => {
                self.last_pointer = point;
                self.dispatch_pointer(PointerPhase::Move, 0);
            }
            Message::PointerDown => self.dispatch_pointer(PointerPhase::Down, 0),
            Message::PointerUp => self.dispatch_pointer(PointerPhase::Up, 0),
            Message::PointerScroll(delta) => {
                let (delta_y, line_delta) = match delta {
                    iced::mouse::ScrollDelta::Lines { y, .. } => (y, true),
                    iced::mouse::ScrollDelta::Pixels { y, .. } => (y, false),
                };
                let document = self.document.document();
                let _ = RuntimeInputAdapter::default().dispatch(
                    self.document.context_mut(),
                    document,
                    &InputEvent::Wheel {
                        x: self.last_pointer.x,
                        y: self.last_pointer.y,
                        delta_x: 0.0,
                        delta_y,
                        line_delta,
                        modifiers: InputModifiers::default(),
                    },
                );
                self.flush_scene();
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        iced::event::listen_with(|event, _status, _id| match event {
            Event::Window(iced::window::Event::Resized(_))
            | Event::Window(iced::window::Event::Rescaled(_))
            | Event::Window(iced::window::Event::Opened { .. }) => Some(Message::Reflow),
            Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) => match key {
                Key::Named(keyboard::key::Named::ArrowRight) => Some(Message::Next),
                Key::Named(keyboard::key::Named::ArrowLeft) => Some(Message::Prev),
                Key::Character(ch) if ch.eq_ignore_ascii_case("t") => Some(Message::Theme),
                Key::Character(ch) if ch == "[" => Some(Message::Prev),
                Key::Character(ch) if ch == "]" => Some(Message::Next),
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

    fn view(&self) -> Element<'_, Message> {
        let colors = self.theme.tokens().colors;
        let scene = self.document.shared_scene();
        let clear_color = [
            colors.surface.r,
            colors.surface.g,
            colors.surface.b,
            colors.surface.a,
        ];
        let pointer = resize_pointer(&self.document, self.last_pointer);
        let iced_view: Element<'_, Message> = IcedSceneView::new(self.document.scene(), PANEL)
            .map(|view| view.pointer_interaction(pointer))
            .map_or_else(
                |_| {
                    space()
                        .width(Length::Fixed(PANEL.width))
                        .height(Length::Fixed(PANEL.height))
                        .into()
                },
                Into::into,
            );
        let host_view = iced::widget::shader(SceneWgpuProgram {
            scene,
            clear_color,
            pointer,
        })
        .width(Length::Fixed(PANEL.width))
        .height(Length::Fixed(PANEL.height));
        column![
            text(format!(
                "{}   Left/Right or 1-9   T theme ({})",
                self.case.label(),
                match self.theme {
                    ThemeMode::Dark => "dark",
                    ThemeMode::Light => "light",
                }
            ))
            .size(13)
            .color(colors.muted),
            row![
                panel(colors, interact(iced_view)),
                panel(colors, interact(host_view.into())),
            ]
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

fn panel<'a>(colors: nana_ui::Colors, content: Element<'a, Message>) -> Element<'a, Message> {
    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_| iced::widget::container::Style::default().background(colors.surface))
        .into()
}

fn interact(content: Element<'_, Message>) -> Element<'_, Message> {
    mouse_area(content)
        .on_move(Message::PointerMove)
        .on_press(Message::PointerDown)
        .on_release(Message::PointerUp)
        .on_scroll(Message::PointerScroll)
        .into()
}

fn remount(
    document: &mut RuntimeDocument,
    case: Case,
    theme: ThemeMode,
    graph: &GraphModel,
    graph_viewport: GraphViewport,
    graph_selection: Option<&GraphSelection>,
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
    match case {
        Case::GraphCanvas => {
            document.context_mut().create_component(
                document_id,
                RuntimeGraphCanvas::new("ab", graph.clone())
                    .viewport(graph_viewport)
                    .selection(graph_selection.cloned()),
            )?;
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
        }
        Case::DockPanel => {
            let body = bare(document, "Inspector")?;
            let panel = document.context_mut().create_component(
                document_id,
                DockPanel::new().padding(10.0).content(body.stable_id()),
            )?;
            document.context_mut().append_child(panel, body)?;
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
        }
        Case::AppTitleBar => {
            document
                .context_mut()
                .create_component(document_id, AppTitleBar::new("NanaUI"))?;
        }
        Case::SettingsPage => {
            let content = bare(document, "Appearance content")?;
            let (model, state) = ab_settings();
            let page = document.context_mut().create_component(
                document_id,
                SettingsPage::new(model.clone(), state.clone()).content(content.stable_id()),
            )?;
            document.context_mut().append_child(page, content)?;
            document.context_mut().assemble_settings_page(page)?;
        }
        Case::Calendar => {
            document.context_mut().create_component(
                document_id,
                RuntimeCalendarHeatmap::new(ab_calendar_data()).label("活动"),
            )?;
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

fn resize_pointer(document: &RuntimeDocument, point: Point) -> iced::mouse::Interaction {
    let context = document.context();
    let document_id = document.document();
    let handle = context
        .split_handle_near(document_id, point.x, point.y)
        .or_else(|| context.dock_handle_near(document_id, point.x, point.y));
    let Some(handle) = handle else {
        return iced::mouse::Interaction::None;
    };
    let Some(bounds) = context.world().layout_box(handle) else {
        return iced::mouse::Interaction::None;
    };
    if bounds.width <= bounds.height {
        iced::mouse::Interaction::ResizingHorizontally
    } else {
        iced::mouse::Interaction::ResizingVertically
    }
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

#[derive(Debug, Clone)]
struct SceneWgpuProgram {
    scene: Arc<UiScene>,
    clear_color: [f32; 4],
    pointer: iced::mouse::Interaction,
}

impl<Message> shader::Program<Message> for SceneWgpuProgram {
    type State = ();
    type Primitive = SceneWgpuPrimitive;

    fn draw(
        &self,
        _state: &Self::State,
        _cursor: iced::mouse::Cursor,
        _bounds: Rectangle,
    ) -> Self::Primitive {
        SceneWgpuPrimitive {
            scene: Arc::clone(&self.scene),
            clear_color: self.clear_color,
        }
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        _bounds: Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> iced::mouse::Interaction {
        self.pointer
    }
}

#[derive(Debug, Clone)]
struct SceneWgpuPrimitive {
    scene: Arc<UiScene>,
    clear_color: [f32; 4],
}

struct SceneWgpuPipeline {
    painter: Mutex<SceneWgpuPainter>,
    renderers: SceneGpuRendererRegistry,
    frame: Mutex<Option<PreparedPaint>>,
}

struct PreparedPaint {
    scene: Arc<UiScene>,
    viewport: ScenePaintViewport,
}

impl shader::Primitive for SceneWgpuPrimitive {
    type Pipeline = SceneWgpuPipeline;

    fn prepare(
        &self,
        pipeline: &mut Self::Pipeline,
        _device: &wgpu::Device,
        _queue: &wgpu::Queue,
        bounds: &Rectangle,
        viewport: &Viewport,
    ) {
        let scale = viewport.scale_factor();
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        let physical = viewport.physical_size();
        let paint_viewport = ScenePaintViewport {
            logical_size: [bounds.width, bounds.height],
            physical_size: [physical.width, physical.height],
            scale_factor: scale,
            scene_origin: [0.0, 0.0],
            target_origin: [bounds.x, bounds.y],
            clear_color: self.clear_color,
            clear: false,
        };
        let mut frame = pipeline
            .frame
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *frame = Some(PreparedPaint {
            scene: Arc::clone(&self.scene),
            viewport: paint_viewport,
        });
    }

    fn draw(&self, _pipeline: &Self::Pipeline, _render_pass: &mut wgpu::RenderPass<'_>) -> bool {
        false
    }

    fn render(
        &self,
        pipeline: &Self::Pipeline,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        _clip_bounds: &Rectangle<u32>,
    ) {
        let frame = pipeline
            .frame
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(frame) = frame.as_ref() else {
            return;
        };
        let viewport = ScenePaintViewport {
            logical_size: frame.viewport.logical_size,
            physical_size: frame.viewport.physical_size,
            scale_factor: frame.viewport.scale_factor,
            scene_origin: frame.viewport.scene_origin,
            target_origin: frame.viewport.target_origin,
            clear_color: frame.viewport.clear_color,
            clear: frame.viewport.clear,
        };
        let mut painter = pipeline
            .painter
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = painter.paint(
            &frame.scene,
            encoder,
            target,
            viewport,
            None,
            Some(&pipeline.renderers),
        );
    }
}

impl shader::Pipeline for SceneWgpuPipeline {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        Self {
            painter: Mutex::new(SceneWgpuPainter::new(device, queue, format)),
            renderers: default_scene_gpu_renderers_with_host(
                Arc::new(device.clone()),
                Arc::new(queue.clone()),
            ),
            frame: Mutex::new(None),
        }
    }
}
