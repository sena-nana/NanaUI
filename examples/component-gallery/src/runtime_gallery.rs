use std::fmt;
use std::sync::{Arc, Mutex};

use nana_ui::runtime::view::{EntityRef, IntoView, detached, entity_ref, widget, with_refs};
use nana_ui::runtime::{
    Activate, AppShell, AppTitleBar, Avatar, Button, CalendarHeatmap, CalendarHeatmapDatum,
    CalendarHeatmapEvent, Card, Chart, ChartEvent, Checkbox, Chip, ControlSize, DesktopShell,
    DiffHunk, DiffLine, DiffView, DockFloatingSurface, DocumentId, DropAccepts, Dropdown,
    DropdownEvent, DropdownOption, EmptyState, Entity, FileDropEvent, FrameworkError, GraphCanvas,
    GraphCanvasEvent, GraphMinimap, GraphMinimapEvent, GraphSize, IconButton, InteractiveCard,
    LabeledValue, LayoutViewport, LengthSpec, LevelMeter, ListItem, ListItemSlots, NativeMarkdown,
    NodeStyle, OverlayHost, PaneChrome, PaneChromeAction, PaneChromeActionKind, PaneTree,
    PaneTreeNode, Popover, PopoverClosed, PopoverToggled, PositionSpec, Progress, RangeInput,
    RichTextEvent, RuntimeDocument, ScrollAxes, ScrollView, SearchDropdown, SearchDropdownEvent,
    SearchDropdownOption, SegmentedControl, SegmentedOption, SegmentedSelectionRequested,
    SelectionState, SemanticColorRole, SidebarFooter, SidebarFooterButton, SidebarFrame,
    SidebarRow, SidebarRowIcon, SidebarRowState, SidebarSection, Skeleton, Spinner, StableNodeId,
    StatusBadge, StatusTone, Switch, TabOption, Tabs, TabsEvent, TerminalScreen, TerminalView,
    Text, TextArea, TextChanged, TextInput, Thumbnail, Toast, ToastTone, ToggleChanged, TreeNode,
    TreeSelectionRequested, TreeView, TreeViewEvent, ValidationIntent, ValidationMessage,
    WorkspaceModel, XYPad, XYPadEvent,
};
use nana_ui::theme::{SpacingStep, ThemeToken, space, type_scale};
use nana_ui::{ButtonKind, CardKind, Icon, NanaTextShaper, RegionId, WorkspaceAction};
use nana_ui_platform::InputPayload;

use super::runtime_host::{
    DEFAULT_VIEWPORT, HostStack, RuntimeChrome, RuntimeSceneInput, ScriptedInput,
    apply_title_bar_maximized, apply_workspace_corners, bind_event, event_point, hugging_text,
    labeled_text, node_is_or_under, queue, reconcile_children, runtime_input_event,
    search_command_button, sidebar_toggle_button, styled_text, take_pending, theme_toggle_button,
};
use super::{
    GalleryChart, GalleryDock, GalleryMessage, GallerySection, GalleryState, SurfaceView,
    section_label,
};

type SidebarMount = (
    Entity<SidebarFrame>,
    [Entity<SidebarRow>; SECTION_COUNT],
    Entity<SidebarFooterButton>,
);
type RichTextMount = (
    Entity<ScrollView>,
    Entity<NativeMarkdown>,
    Entity<nana_ui::runtime::Text>,
    Entity<Card>,
    Entity<nana_ui::runtime::Text>,
    Entity<TextArea>,
    Entity<TerminalView>,
    Entity<DiffView>,
);
type GraphMount = (
    Entity<HostStack>,
    Entity<GraphCanvas>,
    Entity<GraphMinimap>,
    Entity<nana_ui::runtime::Text>,
    Entity<Button>,
);

const GALLERY_DOCUMENT: u64 = 2;

const SECTION_COUNT: usize = 7;

/// The sidebar's pages in stable route/entity order. The sidebar projects
/// these rows into task-oriented groups without changing their identities.
const SECTIONS: [(GallerySection, Icon); SECTION_COUNT] = [
    (GallerySection::Controls, Icon::Sparkles),
    (GallerySection::Surfaces, Icon::Folder),
    (GallerySection::Feedback, Icon::MessageSquarePlus),
    (GallerySection::Data, Icon::Chart),
    (GallerySection::RichText, Icon::File),
    (GallerySection::Graph, Icon::Nodes),
    (GallerySection::Workspace, Icon::Workspace),
];
/// Rows of the list card: label, trailing detail, disabled.
const LIST_ITEMS: [(&str, &str, bool); 6] = [
    ("README.md", "2 KB", false),
    ("Cargo.toml", "1 KB", false),
    ("main.rs", "8 KB", false),
    ("lib.rs", "24 KB", false),
    ("assets", "12 项", false),
    ("target", "不可用", true),
];

/// The tree's files, offered again as chips under it.
const TREE_SHORTCUTS: [(&str, &str); 3] = [
    ("src/lib.rs", "lib.rs"),
    ("src/main.rs", "main.rs"),
    ("README.md", "README.md"),
];

const SURFACE_CARDS: [(&str, &str); 3] = [
    ("默认卡片", "普通内容容器"),
    ("交互卡片", "点按选中"),
    ("禁用卡片", "不可进行操作"),
];

const DOCK_PANELS: [(&str, &str, &str); 8] = [
    ("gallery.primary", "Primary Content", "不可移动的主内容节点"),
    ("gallery.navigation", "Section A", "工作区导航"),
    ("gallery.assets", "Asset", "应用提供的资源内容"),
    ("gallery.inspector", "Selection", "应用提供的检查器内容"),
    ("gallery.outline", "Outline", "当前内容的结构投影"),
    ("gallery.console", "Console", "应用运行输出"),
    ("gallery.problems", "Problems", "应用诊断列表"),
    ("gallery.output", "Output", "应用提供的输出内容"),
];

const DOCK_TITLES: [(&str, &str); 8] = [
    ("gallery.primary", "Primary"),
    ("gallery.navigation", "Navigation"),
    ("gallery.assets", "Assets"),
    ("gallery.inspector", "Inspector"),
    ("gallery.outline", "Outline"),
    ("gallery.console", "Console"),
    ("gallery.problems", "Problems"),
    ("gallery.output", "Output"),
];

/// Gallery snapshot is 1280×800. DesktopShell chrome: title 36 +
/// PrimaryToolbar 34 + Diagnostics 180 leaves the primary region 550; the
/// dock fills what the tool bar leaves and never goes below this.
const WORKSPACE_CANVAS_PADDING: f32 = 8.0;
const WORKSPACE_CANVAS_GAP: f32 = 8.0;
const WORKSPACE_DOCK_MIN_HEIGHT: f32 = 240.0;
const WORKSPACE_POPUP_WIDTH: f32 = 340.0;
const RUNTIME_DOCK_FLOAT_WIDTH: f32 = 360.0;
const RUNTIME_DOCK_FLOAT_HEIGHT: f32 = 280.0;

pub(super) struct GalleryRuntime {
    document: RuntimeDocument,
    shell: Entity<DesktopShell>,
    sidebar_toggle: Entity<IconButton>,
    search_button: Entity<IconButton>,
    theme_button: Entity<IconButton>,
    title_leading: Entity<HostStack>,
    title_center: Entity<nana_ui::runtime::Text>,
    title_trailing: Entity<HostStack>,
    context_label: Entity<nana_ui::runtime::Text>,
    sidebar: Entity<SidebarFrame>,
    sidebar_rows: [Entity<SidebarRow>; SECTION_COUNT],
    settings_footer: Entity<SidebarFooterButton>,
    controls: ControlsTree,
    surfaces: SurfacesTree,
    feedback: FeedbackTree,
    data: DataTree,
    rich_text_root: Entity<ScrollView>,
    rich_text: Entity<NativeMarkdown>,
    link_status: Entity<nana_ui::runtime::Text>,
    #[cfg_attr(not(test), allow(dead_code))]
    drop: Entity<Card>,
    drop_hint: Entity<nana_ui::runtime::Text>,
    #[cfg_attr(not(test), allow(dead_code))]
    code_editor: Entity<TextArea>,
    #[cfg_attr(not(test), allow(dead_code))]
    terminal: Entity<TerminalView>,
    #[cfg_attr(not(test), allow(dead_code))]
    diff: Entity<DiffView>,
    graph_root: Entity<HostStack>,
    graph: Entity<GraphCanvas>,
    graph_minimap: Entity<GraphMinimap>,
    graph_selection: Entity<nana_ui::runtime::Text>,
    _graph_reset: Entity<Button>,
    workspace: WorkspaceTree,
    inspector: InspectorTree,
    _bottom_collapse: Entity<Button>,
    _toolbar_reset: Entity<Button>,
    last_viewport: LayoutViewport,
    chrome: RuntimeChrome,
    scripted: ScriptedInput,
    pending: Arc<Mutex<Vec<GalleryMessage>>>,
    text: NanaTextShaper,
}

struct ControlsTree {
    root: Entity<ScrollView>,
    loading: Entity<Button>,
    clicks: Entity<nana_ui::runtime::Text>,
    segmented: [Entity<SegmentedControl>; 3],
    segmented_on: [Entity<SegmentedOption>; 3],
    segmented_off: [Entity<SegmentedOption>; 3],
    inputs: [Entity<TextInput>; 3],
    secure: Entity<TextInput>,
    dropdowns: [Entity<Dropdown>; 3],
    field_status: Entity<nana_ui::runtime::Text>,
    selection_status: Entity<nana_ui::runtime::Text>,
    checkbox: Entity<Checkbox>,
    switch: Entity<Switch>,
    range: Entity<nana_ui::runtime::RangeField>,
    meter: Entity<LevelMeter>,
    search: Entity<SearchDropdown>,
    textarea: Entity<TextArea>,
    editor_status: Entity<nana_ui::runtime::Text>,
    xy_pad: Entity<XYPad>,
    xy_label: Entity<nana_ui::runtime::Text>,
}

struct SurfacesTree {
    root: Entity<ScrollView>,
    tabs: Entity<Tabs>,
    surface_row: Entity<HostStack>,
    overview: [Entity<Card>; 3],
    cards: [Entity<InteractiveCard>; 3],
    tree: Entity<TreeView>,
    chips: [Entity<Chip>; 3],
    pane: Entity<PaneChrome>,
    pane_tabs: Entity<nana_ui::runtime::Text>,
    pane_tree: Entity<PaneTree>,
    pane_empty: Entity<nana_ui::runtime::Text>,
    pane_editor: Entity<nana_ui::runtime::Text>,
    pane_left: Entity<nana_ui::runtime::Text>,
    pane_right: Entity<nana_ui::runtime::Text>,
    pane_split: Entity<Button>,
    pane_close: Entity<IconButton>,
    labeled: Entity<LabeledValue>,
    list_items: Vec<Entity<ListItem>>,
    list_leads: Vec<Entity<nana_ui::runtime::Text>>,
    list_labels: Vec<Entity<nana_ui::runtime::Text>>,
    list_trails: Vec<Entity<nana_ui::runtime::Text>>,
}

struct FeedbackTree {
    root: Entity<ScrollView>,
    progress: Entity<Progress>,
    progress_label: Entity<nana_ui::runtime::Text>,
    spinner: Entity<Spinner>,
    skeleton: Entity<Skeleton>,
    badge: Entity<StatusBadge>,
    _validation: Entity<ValidationMessage>,
    toast: Entity<Toast>,
    start: Entity<Button>,
    dialog: Entity<Button>,
    context: Entity<Button>,
    _image: Entity<Button>,
    popover: Entity<Popover>,
    popover_action: Entity<Button>,
}

struct DataTree {
    root: Entity<ScrollView>,
    calendar: Entity<CalendarHeatmap>,
    calendar_status: Entity<nana_ui::runtime::Text>,
    trend_status: Entity<nana_ui::runtime::Text>,
    share_status: Entity<nana_ui::runtime::Text>,
}

struct WorkspaceTree {
    root: Entity<HostStack>,
    dock: Entity<nana_ui::runtime::Dock>,
    panels: Vec<(String, Entity<HostStack>)>,
    lock: Entity<Button>,
    hide: Entity<Button>,
    _reset: Entity<Button>,
    status: Entity<nana_ui::runtime::Text>,
}

struct InspectorTree {
    slot: Entity<HostStack>,
    _root: Entity<HostStack>,
    _collapse: Entity<Button>,
    radius: Entity<nana_ui::runtime::RangeField>,
    corners: Entity<Switch>,
}

impl fmt::Debug for GalleryRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GalleryRuntime")
            .field("shell", &self.shell.stable_id())
            .field("last_viewport", &self.last_viewport)
            .finish_non_exhaustive()
    }
}

impl GalleryRuntime {
    fn mount(state: &GalleryState) -> Result<Self, FrameworkError> {
        let pending = Arc::new(Mutex::new(Vec::new()));
        let mut document =
            RuntimeDocument::new(DocumentId::new(GALLERY_DOCUMENT).expect("gallery document id"));
        let document_id = document.document();
        let context = document.context_mut();
        let _ = context.set_preset_theme(state.theme);

        let (sidebar, sidebar_rows, settings_footer) = mount_sidebar(context, document_id, state)?;
        let controls = mount_controls(context, document_id, state, &pending)?;
        let surfaces = mount_surfaces(context, document_id, state, &pending)?;
        let feedback = mount_feedback(context, document_id, state, &pending)?;
        let data = mount_data(context, document_id, state, &pending)?;
        let (rich_text_root, rich_text, link_status, drop, drop_hint, code_editor, terminal, diff) =
            mount_rich_text(context, document_id, state, &pending)?;
        let (graph_root, graph, graph_minimap, graph_selection, graph_reset) =
            mount_graph(context, document_id, state, &pending)?;
        let workspace = mount_workspace(context, document_id, state, &pending)?;
        let inspector = mount_inspector(context, document_id, state, &pending)?;
        let (bottom, bottom_collapse) = mount_bottom(context, document_id, &pending)?;
        let (toolbar, toolbar_reset) = mount_toolbar(context, document_id, &pending)?;

        let sidebar_collapsed = state
            .workspace
            .layout()
            .region(&RegionId::Resources)
            .is_some_and(nana_ui::RegionState::collapsed_value);
        let primary = section_root(
            state.section,
            &controls,
            &surfaces,
            &feedback,
            &data,
            rich_text_root,
            graph_root,
            &workspace,
        );
        let refs = std::cell::Cell::new(None);
        let view = context.mount_view_root(document_id, || {
            let icons = [entity_ref(), entity_ref(), entity_ref()];
            let [toggle, search, theme] = icons;
            let (leading, trailing) = (entity_ref(), entity_ref());
            let (center, label) = (entity_ref(), entity_ref());
            refs.set(Some((icons, leading, center, trailing, label)));
            let muted = hugging_text(
                section_label(state.section),
                SemanticColorRole::Muted,
                type_scale::HINT,
                type_scale::REGULAR,
            );
            let title = hugging_text(
                "NanaUI Gallery",
                SemanticColorRole::Text,
                type_scale::BODY,
                type_scale::SEMIBOLD,
            );
            widget(
                DesktopShell::from_model(state.workspace.model().clone())
                    .title("NanaUI Gallery")
                    .navigation(sidebar.stable_id())
                    .primary(primary)
                    .inspector(inspector.slot.stable_id())
                    .bottom(bottom.stable_id())
                    .region(RegionId::PrimaryToolbar, toolbar.stable_id()),
            )
            .title_leading(
                widget(HostStack::leading_row(0.0))
                    .entity_ref(leading)
                    .children(widget(sidebar_toggle_button(sidebar_collapsed)).entity_ref(toggle)),
            )
            .title_center(widget(title).entity_ref(center))
            .title_trailing(widget(HostStack::row(6.0)).entity_ref(trailing).children((
                widget(muted).entity_ref(label),
                widget(search_command_button()).entity_ref(search),
                widget(theme_toggle_button(state.theme)).entity_ref(theme),
            )))
        })?;
        let shell = view.root().expect("the shell view has its shell");
        let (icons, leading, center, trailing, label) = refs.get().expect("the shell view ran");
        let built = "the shell view built every node";
        let [sidebar_toggle, search_button, theme_button] =
            icons.map(|icon| icon.get().expect(built));
        let (title_leading, title_center) =
            (leading.get().expect(built), center.get().expect(built));
        let (title_trailing, context_label) =
            (trailing.get().expect(built), label.get().expect(built));
        context.assemble_dock(workspace.dock)?;

        bind_event(
            context,
            sidebar_toggle,
            Arc::clone(&pending),
            |event: &Activate| {
                let _ = event;
                GalleryMessage::Workspace(WorkspaceAction::ToggleRegion(RegionId::Resources))
            },
        )?;
        bind_event(
            context,
            search_button,
            Arc::clone(&pending),
            |event: &Activate| {
                let _ = event;
                GalleryMessage::ToggleCommandPalette
            },
        )?;
        bind_event(
            context,
            theme_button,
            Arc::clone(&pending),
            |event: &Activate| {
                let _ = event;
                GalleryMessage::ToggleTheme
            },
        )?;
        bind_event(
            context,
            settings_footer,
            Arc::clone(&pending),
            |event: &Activate| {
                let _ = event;
                GalleryMessage::OpenSettings
            },
        )?;
        for (index, row) in sidebar_rows.iter().copied().enumerate() {
            let section = SECTIONS[index].0;
            bind_event(
                context,
                row,
                Arc::clone(&pending),
                move |event: &Activate| {
                    let _ = event;
                    GalleryMessage::SelectSection(section)
                },
            )?;
        }

        let (width, height) = state.gallery_viewport_size();
        let last_viewport = LayoutViewport::new(width, height);
        let mut text = NanaTextShaper::default();
        let _ = document.flush(last_viewport, &mut text);

        Ok(Self {
            document,
            shell,
            sidebar_toggle,
            search_button,
            theme_button,
            title_leading,
            title_center,
            title_trailing,
            context_label,
            sidebar,
            sidebar_rows,
            settings_footer,
            controls,
            surfaces,
            feedback,
            data,
            rich_text_root,
            rich_text,
            link_status,
            drop,
            drop_hint,
            code_editor,
            terminal,
            diff,
            graph_root,
            graph,
            graph_minimap,
            graph_selection,
            _graph_reset: graph_reset,
            workspace,
            inspector,
            _bottom_collapse: bottom_collapse,
            _toolbar_reset: toolbar_reset,
            last_viewport,
            chrome: RuntimeChrome::default(),
            scripted: ScriptedInput::default(),
            pending,
            text,
        })
    }

    fn sync(&mut self, state: &GalleryState) {
        let context = self.document.context_mut();
        let _ = context.set_preset_theme(state.theme);
        let sidebar_collapsed = state
            .workspace
            .layout()
            .region(&RegionId::Resources)
            .is_some_and(nana_ui::RegionState::collapsed_value);
        let _ = context.update_component(self.sidebar_toggle, |button, _| {
            *button = sidebar_toggle_button(sidebar_collapsed);
        });
        let _ = context.update_component(self.search_button, |button, _| {
            *button = search_command_button();
        });
        let _ = context.update_component(self.theme_button, |button, _| {
            *button = theme_toggle_button(state.theme);
        });
        let _ = context.update_component(self.context_label, |label, _| {
            *label = hugging_text(
                section_label(state.section),
                SemanticColorRole::Muted,
                type_scale::HINT,
                type_scale::REGULAR,
            );
        });
        for (index, row) in self.sidebar_rows.iter().copied().enumerate() {
            let active = state.section == SECTIONS[index].0;
            let _ = context.update_component(row, |row, _| {
                row.state = if active {
                    SidebarRowState::Active
                } else {
                    SidebarRowState::Idle
                };
            });
        }
        let _ = context.update_component(self.settings_footer, |button, _| {
            *button = SidebarFooterButton::new("设置", Icon::Settings);
        });
        sync_controls(context, &self.controls, state);
        sync_surfaces(context, &self.surfaces, state);
        sync_feedback(context, &self.feedback, state);
        sync_data(context, &self.data, state);
        let _ = context.update_component(self.rich_text, |markdown, _| {
            *markdown = state.markdown.clone();
        });
        let _ = context.assemble_markdown(self.rich_text);
        let _ = context.update_component(self.link_status, |label, _| {
            *label = link_status_text(state);
        });
        let _ = context.update_component(self.drop_hint, |label, _| {
            *label = gallery_drop_hint(state);
        });
        let _ = context.update_component(self.graph, |canvas, _| {
            canvas.set_model(state.graph.clone());
            canvas.set_viewport(state.graph_viewport);
            canvas.set_selection(state.graph_selection.clone());
        });
        let _ = context.update_component(self.graph_minimap, |minimap, _| {
            minimap.set_model(state.graph.clone());
            minimap.set_viewport(state.graph_viewport);
        });
        let _ = context.update_component(self.graph_selection, |label, _| {
            *label = hugging_text(
                graph_selection_label(state),
                SemanticColorRole::Muted,
                type_scale::HINT,
                type_scale::REGULAR,
            );
        });
        sync_workspace(context, &self.workspace, state);
        sync_inspector(context, &self.inspector, state);
        let primary = section_root(
            state.section,
            &self.controls,
            &self.surfaces,
            &self.feedback,
            &self.data,
            self.rich_text_root,
            self.graph_root,
            &self.workspace,
        );
        let _ = context.update_component(self.shell, |shell, _| {
            shell.model = state.workspace.model().clone();
            shell.title_leading = Some(self.title_leading.stable_id());
            shell.title_center = Some(self.title_center.stable_id());
            shell.title_trailing = Some(self.title_trailing.stable_id());
            shell.navigation = Some(self.sidebar.stable_id());
            shell.primary = Some(primary);
            shell.inspector = Some(self.inspector.slot.stable_id());
        });
        let _ = context.assemble_dock(self.workspace.dock);
        let _ = context.assemble_desktop_shell(self.shell);
        apply_workspace_corners(
            context,
            self.shell,
            state.appearance.workspace_corners_enabled(),
        );
        apply_title_bar_maximized(context, self.shell, state.window_chrome.is_maximized());
        self.flush(state.gallery_viewport_size());
    }

    fn flush(&mut self, (width, height): (f32, f32)) {
        self.last_viewport = LayoutViewport::new(width, height);
        if let Err(error) = self.document.flush(self.last_viewport, &mut self.text) {
            report_failure("layout flush", &error);
        }
    }

    pub(super) fn runtime_document(&self) -> &RuntimeDocument {
        self.document()
    }

    pub(super) fn runtime_document_mut(&mut self) -> &mut RuntimeDocument {
        self.document_mut()
    }

    pub(super) fn shell(&self) -> Entity<DesktopShell> {
        self.shell
    }

    pub(super) fn overlay_host(&self) -> Option<Entity<OverlayHost>> {
        self.document
            .context()
            .read(self.shell, |shell| {
                shell.overlay.map(Entity::<OverlayHost>::from_stable_id)
            })
            .ok()
            .flatten()
    }

    pub(super) fn pending_sink(&self) -> Arc<Mutex<Vec<GalleryMessage>>> {
        Arc::clone(&self.pending)
    }

    /// Route a file-drag phase the way the window's input source does.
    #[cfg(test)]
    pub(super) fn route_file_drag(
        &mut self,
        kind: nana_ui::FileDragKind,
        paths: &[std::path::PathBuf],
        position: Option<(f32, f32)>,
    ) -> (bool, Vec<GalleryMessage>) {
        let outcome = self.scripted.route(
            &mut self.document,
            InputPayload::FileDrag(nana_ui::FileDragInput {
                kind,
                paths: paths.to_vec(),
                position,
                modifiers: Default::default(),
            }),
        );
        (
            outcome.is_some_and(|outcome| outcome.handled),
            take_pending(&self.pending),
        )
    }

    pub(super) fn flush_viewport(&mut self, size: (f32, f32)) {
        self.flush(size);
    }

    pub(super) fn note_pointer(&mut self, event: &InputPayload) {
        if let Some(point) = event_point(event) {
            self.chrome.last_pointer = point;
        }
    }

    pub(super) fn workspace_model(&self) -> Option<WorkspaceModel> {
        self.document
            .context()
            .read(self.shell, |shell| shell.model.clone())
            .ok()
    }

    pub(super) fn workspace_is_resizing(&self) -> bool {
        self.document
            .context()
            .read(self.shell, |shell| shell.model.is_resizing())
            .unwrap_or(false)
    }

    pub(super) fn take_host_messages(&mut self, event: &InputPayload) -> Vec<GalleryMessage> {
        self.note_pointer(event);
        let extra = self.host_pointer_messages(event);
        let mut messages = take_pending(&self.pending);
        messages.extend(extra);
        if !self.workspace_is_resizing() {
            messages.extend(self.chrome.title_bar_chrome_messages(&self.document, event));
        }
        messages
    }

    fn dispatch(&mut self, event: InputPayload) -> Vec<GalleryMessage> {
        self.note_pointer(&event);
        let extra = self.host_pointer_messages(&event);
        self.scripted.route(&mut self.document, event.clone());
        let mut messages = take_pending(&self.pending);
        messages.extend(extra);
        if !self.workspace_is_resizing() {
            messages.extend(
                self.chrome
                    .title_bar_chrome_messages(&self.document, &event),
            );
        }
        messages
    }

    fn host_pointer_messages(&mut self, event: &InputPayload) -> Vec<GalleryMessage> {
        let InputPayload::Pointer(nana_ui_platform::PointerInput {
            phase,
            button,
            x,
            y,
            ..
        }) = *event
        else {
            return Vec::new();
        };
        let context = self.document.context();
        let target = context.pointer_target(self.document.document(), x, y);
        match phase {
            nana_ui_platform::PointerPhase::Move => {
                let Some(bounds) =
                    scrolled_box(context, self.data.calendar.stable_id(), self.data.root)
                else {
                    return Vec::new();
                };
                let hit = context
                    .read(self.data.calendar, |heatmap| {
                        heatmap.cell_at_in(bounds, x, y)
                    })
                    .ok()
                    .flatten();
                match hit {
                    Some(cell) => vec![GalleryMessage::CalendarHeatmap(
                        CalendarHeatmapEvent::CellMove(cell),
                    )],
                    None if node_is_or_under(
                        context,
                        target.unwrap_or(self.data.calendar.stable_id()),
                        self.data.calendar.stable_id(),
                    ) =>
                    {
                        vec![GalleryMessage::CalendarHeatmap(
                            CalendarHeatmapEvent::CellLeave,
                        )]
                    }
                    None => Vec::new(),
                }
            }
            nana_ui_platform::PointerPhase::Up if button == 0 => {
                let mut messages = Vec::new();
                if let Some(target) = target {
                    for (index, card) in self.surfaces.cards.iter().enumerate() {
                        if index != 2 && node_is_or_under(context, target, card.stable_id()) {
                            messages.push(GalleryMessage::SelectSurfaceCard(index));
                        }
                    }
                    if node_is_or_under(context, target, self.rich_text.stable_id())
                        && let Some(bounds) =
                            scrolled_box(context, self.rich_text.stable_id(), self.rich_text_root)
                        && let Ok(Some(RichTextEvent::LinkActivated(link))) = context
                            .read(self.rich_text, |markdown| markdown.pointer_up(x, y, bounds))
                    {
                        messages.push(GalleryMessage::OpenMarkdownLink(link.to_string()));
                    }
                    if node_is_or_under(context, target, self.workspace.lock.stable_id())
                        && let Ok(locked) = context.read(self.workspace.dock, |dock| dock.locked)
                    {
                        messages.push(GalleryMessage::Dock(GalleryDock::SetLocked(!locked)));
                    }
                    if node_is_or_under(context, target, self.workspace.hide.stable_id()) {
                        let assets_visible = context
                            .read(self.workspace.dock, |dock| {
                                dock.flatten()
                                    .iter()
                                    .any(|id| id.as_ref() == "gallery.assets")
                            })
                            .unwrap_or(true);
                        messages.push(GalleryMessage::Dock(if assets_visible {
                            GalleryDock::Hide(Arc::from("gallery.assets"))
                        } else {
                            GalleryDock::Show(Arc::from("gallery.assets"))
                        }));
                    }
                    if let Ok(Some(selected)) =
                        context.read(self.workspace.dock, |dock| selected_dock_tab(context, dock))
                    {
                        messages.push(GalleryMessage::Dock(GalleryDock::ActivateTab(Arc::from(
                            selected,
                        ))));
                    }
                }
                messages
            }
            nana_ui_platform::PointerPhase::Down if button == 2 => {
                if target.is_some_and(|target| {
                    node_is_or_under(context, target, self.feedback.context.stable_id())
                }) {
                    vec![GalleryMessage::ToggleContextMenu]
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    pub(super) fn document(&self) -> &RuntimeDocument {
        &self.document
    }

    pub(super) fn document_mut(&mut self) -> &mut RuntimeDocument {
        &mut self.document
    }

    #[cfg(test)]
    fn scene_populated(&self) -> bool {
        !self.document.scene().is_empty()
    }

    #[cfg(test)]
    fn markdown_has_mermaid_presenter(&self) -> bool {
        let context = self.document.context();
        let children = context
            .read(self.rich_text, |markdown| {
                markdown.fence_children().to_vec()
            })
            .unwrap_or_default();
        children.iter().any(|id| {
            context
                .world()
                .highlight_request(*id)
                .is_some_and(|request| {
                    request.presenter.as_ref() == NativeMarkdown::MERMAID_PRESENTER
                })
        })
    }

    #[cfg(test)]
    fn drop_target_center(&self) -> Option<(f32, f32)> {
        let bounds = scrolled_box(
            self.document.context(),
            self.drop.stable_id(),
            self.rich_text_root,
        )?;
        Some((
            bounds.x + bounds.width * 0.5,
            bounds.y + bounds.height * 0.5,
        ))
    }

    #[cfg(test)]
    fn drop_hint_text(&self) -> Option<String> {
        self.document
            .context()
            .world()
            .text(self.drop_hint.stable_id())
            .map(str::to_owned)
    }

    #[cfg(test)]
    fn code_editor_gutters(&self) -> Option<(bool, bool, usize, usize)> {
        self.document
            .context()
            .read(self.code_editor, |editor| {
                (
                    editor.line_numbers,
                    editor.minimap,
                    editor.diagnostics.len(),
                    editor.git_gutter.len(),
                )
            })
            .ok()
    }

    #[cfg(test)]
    fn terminal_prompt(&self) -> Option<String> {
        self.document
            .context()
            .read(self.terminal, |terminal| {
                terminal
                    .screen
                    .cells
                    .iter()
                    .filter(|cell| cell.width > 0)
                    .map(|cell| cell.text.as_ref())
                    .collect::<String>()
            })
            .ok()
    }

    #[cfg(test)]
    fn diff_has_review_actions(&self) -> bool {
        use nana_ui::runtime::AccessibilityRole;

        let context = self.document.context();
        let world = context.world();
        let Some(hunk) = context
            .assembled_child(self.diff.stable_id(), "body")
            .and_then(|body| context.assembled_child(body, "hunk-0"))
        else {
            return false;
        };
        ["accept", "reject"].iter().all(|key| {
            context.assembled_child(hunk, key).is_some_and(|id| {
                world
                    .accessibility(id)
                    .is_some_and(|state| state.role == AccessibilityRole::Button)
            })
        })
    }

    #[cfg(test)]
    fn first_dock_handle_drag(&self) -> Option<(nana_ui::LogicalPoint, nana_ui::LogicalPoint)> {
        let context = self.document.context();
        let document = self.document.document();
        context
            .world()
            .document_order(document)
            .into_iter()
            .find_map(|id| {
                if !context.is_dock_handle(id) {
                    return None;
                }
                let bounds = context.world().layout_box(id)?;
                if bounds.width <= 0.0 || bounds.height <= 0.0 {
                    return None;
                }
                let start = nana_ui::LogicalPoint::new(
                    bounds.x + bounds.width / 2.0,
                    bounds.y + bounds.height / 2.0,
                );
                let end = if bounds.width <= bounds.height {
                    nana_ui::LogicalPoint::new(start.x + 40.0, start.y)
                } else {
                    nana_ui::LogicalPoint::new(start.x, start.y + 40.0)
                };
                Some((start, end))
            })
    }
}

impl GalleryState {
    pub(super) fn gallery_viewport_size(&self) -> (f32, f32) {
        self.window_size.unwrap_or(DEFAULT_VIEWPORT)
    }

    pub(super) fn refresh_gallery_runtime(&mut self) {
        let (width, height) = self.gallery_viewport_size();
        if self.workspace.viewport_geometry().logical_size != (width, height) {
            self.workspace
                .update(WorkspaceAction::WindowResized { width, height });
        }
        if self.gallery_runtime.is_none() {
            match GalleryRuntime::mount(self) {
                Ok(runtime) => self.gallery_runtime = Some(runtime),
                Err(error) => {
                    report_failure("gallery mount", &error);
                    return;
                }
            }
        }
        if let Some(mut runtime) = self.gallery_runtime.take() {
            runtime.sync(self);
            self.gallery_runtime = Some(runtime);
        }
    }

    pub(super) fn handle_gallery_runtime_input(&mut self, input: RuntimeSceneInput) {
        if self.gallery_runtime.is_none() {
            self.refresh_gallery_runtime();
        }
        let Some(mut runtime) = self.gallery_runtime.take() else {
            return;
        };
        if let RuntimeSceneInput::PointerMove(point)
        | RuntimeSceneInput::PointerDown { point, .. }
        | RuntimeSceneInput::PointerUp { point, .. } = input
        {
            runtime.chrome.last_pointer = point;
        }
        let event = runtime_input_event(&input, runtime.chrome.last_pointer);
        let messages = runtime.dispatch(event);
        self.persist_runtime_dock_workspace(&runtime);
        self.gallery_runtime = Some(runtime);
        let empty = messages.is_empty();
        for message in messages {
            self.update(message);
        }
        if empty
            && !self.settings_open
            && let Some(mut runtime) = self.gallery_runtime.take()
        {
            runtime.flush(self.gallery_viewport_size());
            self.gallery_runtime = Some(runtime);
        }
    }

    /// Copy live Runtime `Dock.root` into product [`DockWorkspace`]. Same tree,
    /// not a second split-ratio authority.
    pub(super) fn persist_runtime_dock_workspace(&mut self, runtime: &GalleryRuntime) {
        let Ok((root, hidden, locked)) = runtime
            .document
            .context()
            .read(runtime.workspace.dock, |dock| {
                (dock.root.clone(), dock.hidden.clone(), dock.locked)
            })
        else {
            return;
        };

        for id in floated_runtime_dock_ids(&self.dock.main, &root, &hidden) {
            if id.as_ref() == super::DOCK_CENTER {
                continue;
            }
            self.apply_gallery_dock(GalleryDock::Float {
                id,
                x: runtime.chrome.last_pointer.x,
                y: runtime.chrome.last_pointer.y,
                width: RUNTIME_DOCK_FLOAT_WIDTH,
                height: RUNTIME_DOCK_FLOAT_HEIGHT,
            });
        }

        let root = dock_tree_without_contents(&root);
        if self.dock.main != root {
            self.dock.main = root;
        }
        if self.dock.hidden != hidden {
            self.dock.hidden = hidden;
        }
        if self.dock_locked != locked {
            self.dock_locked = locked;
        }
        self.persist_dock();
    }

    #[cfg(test)]
    pub(crate) fn gallery_runtime_scene_populated(&self) -> bool {
        self.gallery_runtime
            .as_ref()
            .is_some_and(GalleryRuntime::scene_populated)
    }

    #[cfg(test)]
    pub(crate) fn gallery_runtime_markdown_has_mermaid_presenter(&self) -> bool {
        self.gallery_runtime
            .as_ref()
            .is_some_and(GalleryRuntime::markdown_has_mermaid_presenter)
    }

    #[cfg(test)]
    pub(crate) fn gallery_dispatch_file_drag(
        &mut self,
        kind: nana_ui::FileDragKind,
        paths: &[std::path::PathBuf],
        position: Option<(f32, f32)>,
    ) -> bool {
        let (changed, messages) = {
            let Some(runtime) = self.gallery_runtime.as_mut() else {
                return false;
            };
            runtime.route_file_drag(kind, paths, position)
        };
        for message in messages {
            self.update(message);
        }
        changed
    }

    #[cfg(test)]
    pub(crate) fn gallery_drop_target_center(&self) -> Option<(f32, f32)> {
        self.gallery_runtime
            .as_ref()
            .and_then(GalleryRuntime::drop_target_center)
    }

    #[cfg(test)]
    pub(crate) fn gallery_drop_hint_text(&self) -> Option<String> {
        self.gallery_runtime
            .as_ref()
            .and_then(GalleryRuntime::drop_hint_text)
    }

    #[cfg(test)]
    pub(crate) fn gallery_code_editor_gutters(&self) -> Option<(bool, bool, usize, usize)> {
        self.gallery_runtime
            .as_ref()
            .and_then(GalleryRuntime::code_editor_gutters)
    }

    #[cfg(test)]
    pub(crate) fn gallery_terminal_prompt(&self) -> Option<String> {
        self.gallery_runtime
            .as_ref()
            .and_then(GalleryRuntime::terminal_prompt)
    }

    #[cfg(test)]
    pub(crate) fn gallery_diff_has_review_actions(&self) -> bool {
        self.gallery_runtime
            .as_ref()
            .is_some_and(GalleryRuntime::diff_has_review_actions)
    }

    #[cfg(test)]
    pub(crate) fn gallery_runtime_dock_handle_drag(
        &self,
    ) -> Option<(nana_ui::LogicalPoint, nana_ui::LogicalPoint)> {
        self.gallery_runtime
            .as_ref()
            .and_then(GalleryRuntime::first_dock_handle_drag)
    }

    pub fn runtime_document(&self) -> Option<&RuntimeDocument> {
        if self.settings_open {
            self.settings_runtime
                .as_ref()
                .map(super::runtime_settings::GallerySettingsRuntime::runtime_document)
        } else {
            self.gallery_runtime
                .as_ref()
                .map(GalleryRuntime::runtime_document)
        }
    }
}

pub(super) struct DockWindowRuntime {
    document: RuntimeDocument,
    dock: Entity<nana_ui::runtime::Dock>,
    panels: Vec<(String, Entity<HostStack>)>,
    text: NanaTextShaper,
}

impl fmt::Debug for DockWindowRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DockWindowRuntime")
            .field("dock", &self.dock.stable_id())
            .finish_non_exhaustive()
    }
}

impl DockWindowRuntime {
    pub(super) fn mount(
        state: &GalleryState,
        surface: &DockFloatingSurface,
    ) -> Result<Self, FrameworkError> {
        let document_id = DocumentId::new(100 + surface.window_key())
            .or_else(|| DocumentId::new(100))
            .expect("dock document id");
        let mut document = RuntimeDocument::new(document_id);
        let context = document.context_mut();
        let _ = context.set_preset_theme(state.theme);
        let ids = surface.root.flatten();
        let (_, (dock, panels)) = context.mount_view_root(document_id, || {
            let dock = entity_ref::<nana_ui::runtime::Dock>();
            let panels: Vec<EntityRef<HostStack>> = ids.iter().map(|_| entity_ref()).collect();
            let panel_views = ids
                .iter()
                .zip(&panels)
                .map(|(id, panel)| {
                    let (title, hint) = DOCK_PANELS
                        .iter()
                        .find(|(panel, _, _)| *panel == id.as_ref())
                        .map(|(_, title, hint)| (*title, *hint))
                        .unwrap_or(("Panel", ""));
                    dock_panel(id, title, hint, *panel)
                })
                .collect::<Vec<_>>();
            let view = widget(runtime_dock_from_node(
                state,
                &surface.root,
                &std::collections::HashMap::new(),
            ))
            .entity_ref(dock)
            .children(panel_views);
            with_refs(view, (dock, panels))
        })?;
        let panels = ids.iter().map(|id| id.to_string()).zip(panels).collect();
        let mut text = NanaTextShaper::default();
        let _ = document.flush(
            LayoutViewport::new(surface.width.max(1.0), surface.height.max(1.0)),
            &mut text,
        );
        Ok(Self {
            document,
            dock,
            panels,
            text,
        })
    }

    pub(super) fn sync(&mut self, state: &GalleryState, surface: &DockFloatingSurface) {
        let context = self.document.context_mut();
        let _ = context.set_preset_theme(state.theme);
        let mut contents = std::collections::HashMap::new();
        for (id, panel) in &self.panels {
            contents.insert(id.clone(), panel.stable_id());
        }
        let _ = context.update_component(self.dock, |dock, _| {
            *dock = runtime_dock_from_node(state, &surface.root, &contents);
        });
        let _ = context.assemble_dock(self.dock);
        if let Err(error) = self.document.flush(
            LayoutViewport::new(surface.width.max(1.0), surface.height.max(1.0)),
            &mut self.text,
        ) {
            report_failure("dock surface flush", &error);
        }
    }

    pub(super) fn runtime_document(&self) -> &RuntimeDocument {
        &self.document
    }

    pub(super) fn runtime_document_mut(&mut self) -> &mut RuntimeDocument {
        &mut self.document
    }

    pub(super) fn resize(&mut self, width: f32, height: f32) {
        if let Err(error) = self.document.flush(
            LayoutViewport::new(width.max(1.0), height.max(1.0)),
            &mut self.text,
        ) {
            report_failure("dock surface resize", &error);
        }
    }
}

/// Reports a failure the gallery itself cannot act on.
///
/// Tests panic rather than continue. A swallowed flush leaves every node at its
/// default all-zero layout, and this crate's assertions mostly read *state*
/// rather than layout, so they keep passing against a world that was never laid
/// out -- which is how a real shaping failure hid behind an unrelated `assert!`
/// on macOS CI for ten runs. Failing at the source names the actual error.
#[track_caller]
fn report_failure(context: &str, error: &dyn std::fmt::Debug) {
    if cfg!(test) {
        panic!("gallery {context} failed: {error:?}");
    }
    eprintln!("gallery {context} failed: {error:?}");
}

/// `id`'s layout box where it is drawn: moved by the page's scroll, which
/// layout boxes do not include.
fn scrolled_box(
    context: &nana_ui::runtime::AppContext,
    id: StableNodeId,
    page: Entity<ScrollView>,
) -> Option<nana_ui::runtime::LayoutBox> {
    let mut bounds = context.world().layout_box(id)?;
    if let Some(offset) = context.world().scroll_offset(page.stable_id()) {
        bounds.x -= offset.x;
        bounds.y -= offset.y;
    }
    Some(bounds)
}

#[allow(clippy::too_many_arguments)]
fn section_root(
    section: GallerySection,
    controls: &ControlsTree,
    surfaces: &SurfacesTree,
    feedback: &FeedbackTree,
    data: &DataTree,
    rich_text: Entity<ScrollView>,
    graph: Entity<HostStack>,
    workspace: &WorkspaceTree,
) -> StableNodeId {
    match section {
        GallerySection::Controls => controls.root.stable_id(),
        GallerySection::Surfaces => surfaces.root.stable_id(),
        GallerySection::Feedback => feedback.root.stable_id(),
        GallerySection::Data => data.root.stable_id(),
        GallerySection::RichText => rich_text.stable_id(),
        GallerySection::Graph => graph.stable_id(),
        GallerySection::Workspace => workspace.root.stable_id(),
    }
}

/// Lead above a sidebar group after the first. Groups only read as groups when
/// they stand a step looser than the rows inside them, and `GroupInPane` is the
/// spacing role for groups in a window's column.
const SIDEBAR_GROUP_LEAD: SpacingStep = SpacingStep::Xxxl;

fn sidebar_group_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    let layout = Arc::make_mut(&mut style.layout);
    layout.margin_top = Some(LengthSpec::Px(space::XXXL));
    layout.theme.margin_top = Some(ThemeToken::Space(SIDEBAR_GROUP_LEAD).into());
    style
}

fn mount_sidebar(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    state: &GalleryState,
) -> Result<SidebarMount, FrameworkError> {
    let refs = std::cell::Cell::new(None);
    context.mount_view_detached(document_id, || {
        let frame = entity_ref::<SidebarFrame>();
        let rows: [EntityRef<SidebarRow>; SECTION_COUNT] = std::array::from_fn(|_| entity_ref());
        let settings = entity_ref::<SidebarFooterButton>();
        refs.set(Some((frame, rows, settings)));
        let row_views = SECTIONS
            .iter()
            .zip(rows)
            .enumerate()
            .map(|(index, ((target, icon), row))| {
                let row_state = if state.section == *target {
                    SidebarRowState::Active
                } else {
                    SidebarRowState::Idle
                };
                widget(SidebarRow::new(section_label(*target)).state(row_state))
                    .key(format!("row-{index}"))
                    .entity_ref(row)
                    .child_slot(widget(SidebarRowIcon::new(*icon)), |row, leading| {
                        row.slots(ListItemSlots {
                            leading: Some(leading),
                            content: None,
                            trailing: None,
                        })
                    })
            })
            .collect::<Vec<_>>();

        // Keep the seven route rows stable while grouping them by the job users
        // are trying to accomplish. This is intentionally a sidebar-only
        // projection: the page roots and their state stay independent.
        let mut interaction = Vec::new();
        let mut browsing = Vec::new();
        let mut analysis = Vec::new();
        let mut feedback = Vec::new();
        let mut workspace = Vec::new();
        for (index, row) in row_views.into_iter().enumerate() {
            match index {
                0 => interaction.push(row),
                1 | 4 => browsing.push(row),
                2 => feedback.push(row),
                3 | 5 => analysis.push(row),
                6 => workspace.push(row),
                _ => unreachable!("gallery sidebar row index"),
            }
        }

        widget(SidebarFrame::new())
            .entity_ref(frame)
            .body((
                widget(SidebarSection::new("基础交互").count(interaction.len()))
                    .children(interaction),
                widget(
                    SidebarSection::new("内容浏览")
                        .count(browsing.len())
                        .style(sidebar_group_style()),
                )
                .children(browsing),
                widget(
                    SidebarSection::new("数据分析")
                        .count(analysis.len())
                        .style(sidebar_group_style()),
                )
                .children(analysis),
                widget(
                    SidebarSection::new("反馈与浮层")
                        .count(feedback.len())
                        .style(sidebar_group_style()),
                )
                .children(feedback),
                widget(
                    SidebarSection::new("工作区")
                        .count(workspace.len())
                        .style(sidebar_group_style()),
                )
                .children(workspace),
            ))
            .footer(widget(SidebarFooter::new()).children(
                widget(SidebarFooterButton::new("设置", Icon::Settings)).entity_ref(settings),
            ))
    })?;
    let (frame, rows, settings) = refs.get().expect("the sidebar view ran");
    let built = "the sidebar view built every node";
    Ok((
        frame.get().expect(built),
        rows.map(|row| row.get().expect(built)),
        settings.get().expect(built),
    ))
}

fn mount_controls(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    state: &GalleryState,
    pending: &Arc<Mutex<Vec<GalleryMessage>>>,
) -> Result<ControlsTree, FrameworkError> {
    let sizes = [ControlSize::Small, ControlSize::Medium, ControlSize::Large];
    let (_, refs) = context.mount_view_detached(document_id, || {
        let root = entity_ref::<ScrollView>();
        let buttons: [EntityRef<Button>; 4] = std::array::from_fn(|_| entity_ref());
        let add = entity_ref::<IconButton>();
        let segmented: [EntityRef<SegmentedControl>; 3] = std::array::from_fn(|_| entity_ref());
        let segmented_on: [EntityRef<SegmentedOption>; 3] = std::array::from_fn(|_| entity_ref());
        let segmented_off: [EntityRef<SegmentedOption>; 3] = std::array::from_fn(|_| entity_ref());
        let inputs: [EntityRef<TextInput>; 4] = std::array::from_fn(|_| entity_ref());
        let dropdowns: [EntityRef<Dropdown>; 3] = std::array::from_fn(|_| entity_ref());
        let texts: [EntityRef<Text>; 5] = std::array::from_fn(|_| entity_ref());
        let [
            clicks,
            field_status,
            selection_status,
            editor_status,
            xy_label,
        ] = texts;
        let checkbox = entity_ref::<Checkbox>();
        let switch = entity_ref::<Switch>();
        let range = entity_ref::<nana_ui::runtime::RangeField>();
        let meter = entity_ref::<LevelMeter>();
        let search = entity_ref::<SearchDropdown>();
        let textarea = entity_ref::<TextArea>();
        let xy_pad = entity_ref::<XYPad>();
        let [small, medium, large, loading] = buttons;

        let primary = |label: &'static str, size, kind, button: EntityRef<Button>| {
            widget(Button::new(label).size(size).kind(kind))
                .entity_ref(button)
                .on(queue(pending, |_: &Activate| GalleryMessage::PrimaryAction))
        };
        let buttons_card = showcase(
            "按钮",
            "小、中、大三档尺寸，加载状态与图标按钮。",
            (
                widget(HostStack::leading_row(8.0)).children((
                    primary("小号", ControlSize::Small, ButtonKind::Subtle, small),
                    primary("中号", ControlSize::Medium, ButtonKind::Primary, medium),
                    primary("大号", ControlSize::Large, ButtonKind::Subtle, large),
                    widget(loading_button(state))
                        .entity_ref(loading)
                        .on(queue(pending, |_: &Activate| GalleryMessage::ToggleLoading)),
                    widget(IconButton::new(Icon::Add, "添加").size(ControlSize::Small))
                        .entity_ref(add)
                        .on(queue(pending, |_: &Activate| GalleryMessage::PrimaryAction)),
                )),
                widget(click_count_text(state)).entity_ref(clicks),
            ),
        );

        let segmented_row = widget(HostStack::leading_row(8.0)).children(
            sizes
                .iter()
                .zip(segmented)
                .map(|(size, control)| {
                    widget(SegmentedControl::new().size(*size)).entity_ref(control)
                })
                .collect::<Vec<_>>(),
        );
        // Detached: `set_segmented_options` places them once they exist.
        let segmented_options = sizes
            .iter()
            .enumerate()
            .flat_map(|(index, size)| {
                [
                    widget(SegmentedOption::new("关").size(*size)).entity_ref(segmented_off[index]),
                    widget(SegmentedOption::new("开").size(*size)).entity_ref(segmented_on[index]),
                ]
            })
            .collect::<Vec<_>>();
        let toggles_card = showcase(
            "开关与分段",
            "复选框与三档分段控件共用一个启用状态；启用后才能打开开关。",
            (
                widget(Checkbox::new("启用选项", state.checked))
                    .entity_ref(checkbox)
                    .on(queue(pending, |event: &ToggleChanged| {
                        GalleryMessage::ToggleCheck(event.checked)
                    })),
                widget(Switch::new("允许编辑说明", state.switched).disabled(!state.checked))
                    .entity_ref(switch)
                    .on(queue(pending, |event: &ToggleChanged| {
                        GalleryMessage::ToggleSwitch(event.checked)
                    })),
                segmented_row,
            ),
        );

        let text_changed = || {
            queue(pending, |event: &TextChanged| {
                GalleryMessage::InputChanged(event.value.to_string())
            })
        };
        let input_row = widget(HostStack::fill_row(8.0)).children(
            ["小号", "中号", "大号"]
                .iter()
                .zip(sizes)
                .zip(inputs)
                .map(|((placeholder, size), input)| {
                    flex_cell(
                        widget(
                            TextInput::new(state.input.clone())
                                .placeholder(*placeholder)
                                .size(size)
                                .invalid(state.input.trim().is_empty()),
                        )
                        .entity_ref(input)
                        .on(text_changed()),
                    )
                })
                .collect::<Vec<_>>(),
        );
        let secure = inputs[3];
        let inputs_card = showcase(
            "文本输入与校验",
            "三档输入框共用一个值，留空时标红并给出提示；密钥框隐藏输入内容。",
            (
                input_row,
                widget(
                    TextInput::new(state.input.clone())
                        .placeholder("配对密钥")
                        .secure(true),
                )
                .entity_ref(secure)
                .on(text_changed()),
                widget(field_status_text(state)).entity_ref(field_status),
            ),
        );

        let dropdown_row = widget(HostStack::fill_row(8.0)).children(
            ["小号", "中号", "大号"]
                .iter()
                .zip(sizes)
                .zip(dropdowns)
                .map(|((placeholder, size), dropdown)| {
                    flex_cell(
                        widget(gallery_dropdown(state, placeholder, size))
                            .entity_ref(dropdown)
                            .on(queue(pending, |event: &DropdownEvent<Arc<str>>| {
                                map_dropdown_event(event)
                            })),
                    )
                })
                .collect::<Vec<_>>(),
        );
        let select_card = showcase(
            "下拉与搜索",
            "三个多选下拉共用一份选择；搜索下拉按输入过滤选项。",
            (
                dropdown_row,
                widget(gallery_search(state))
                    .entity_ref(search)
                    .on(queue(pending, |event: &SearchDropdownEvent| {
                        map_search_event(event)
                    })),
                widget(selection_status_text(state)).entity_ref(selection_status),
            ),
        );

        let numbers_card = showcase(
            "数值与滑块",
            "滑块驱动下方的电平表；二维垫同时调整两个参数。",
            (
                widget(fill_range_field(
                    nana_ui::runtime::RangeField::new(f64::from(state.slider), 0.0, 100.0, 1.0)
                        .label("强度")
                        .unit("%"),
                ))
                .entity_ref(range)
                .on(queue(pending, |event: &RangeInput| {
                    GalleryMessage::SetSlider(event.value.round() as u8)
                })),
                widget(LevelMeter::new(f32::from(state.slider) / 100.0)).entity_ref(meter),
                widget(XYPad::new(state.xy_pad).step(0.01).height(112.0))
                    .entity_ref(xy_pad)
                    .on(queue(pending, |event: &XYPadEvent| {
                        GalleryMessage::SetXYPad(*event)
                    })),
                widget(xy_label_text(state)).entity_ref(xy_label),
            ),
        );

        let text_card = showcase(
            "多行文本",
            "勾选「启用选项」并打开开关后可以编辑；少于 4 个字符时提示。",
            (
                widget(gallery_textarea(state))
                    .entity_ref(textarea)
                    .on(queue(pending, |event: &TextChanged| {
                        GalleryMessage::SetEditorText(event.value.to_string())
                    })),
                widget(editor_status_text(state)).entity_ref(editor_status),
            ),
        );

        let view = (
            widget(page_scroll())
                .entity_ref(root)
                .children(widget(page_body()).children((
                    page_header(
                        "操作与输入",
                        "按钮、开关、输入框、下拉与数值控件，每个控件都接到同一份应用状态。",
                    ),
                    card_row((buttons_card, toggles_card)),
                    card_row((inputs_card, select_card)),
                    card_row((numbers_card, text_card)),
                ))),
            segmented_options,
        );
        with_refs(
            view,
            (
                (root, buttons, add, texts),
                (segmented, segmented_on, segmented_off),
                (inputs, dropdowns),
                (checkbox, switch, range, meter, search),
                (textarea, xy_pad),
            ),
        )
    })?;
    let (
        (root, [_small, _medium, _large, loading], _add, texts),
        (segmented, segmented_on, segmented_off),
        (inputs, dropdowns),
        (checkbox, switch, range, meter, search),
        (textarea, xy_pad),
    ) = refs;
    let [
        clicks,
        field_status,
        selection_status,
        editor_status,
        xy_label,
    ] = texts;
    for index in 0..3 {
        let on = segmented_on[index];
        bind_event(
            context,
            segmented[index],
            Arc::clone(pending),
            move |event: &SegmentedSelectionRequested| {
                GalleryMessage::ToggleCheck(event.option == on.stable_id())
            },
        )?;
        context.set_segmented_options(
            segmented[index],
            vec![segmented_off[index], segmented_on[index]],
            Some(if state.checked {
                segmented_on[index]
            } else {
                segmented_off[index]
            }),
        )?;
    }
    let [input_small, input_medium, input_large, secure] = inputs;
    Ok(ControlsTree {
        root,
        loading,
        clicks,
        segmented,
        segmented_on,
        segmented_off,
        inputs: [input_small, input_medium, input_large],
        secure,
        dropdowns,
        field_status,
        selection_status,
        checkbox,
        switch,
        range,
        meter,
        search,
        textarea,
        editor_status,
        xy_pad,
        xy_label,
    })
}

fn mount_surfaces(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    state: &GalleryState,
    pending: &Arc<Mutex<Vec<GalleryMessage>>>,
) -> Result<SurfacesTree, FrameworkError> {
    let selected = SurfaceView::from_index(state.surface_selection.selected());
    let item_open = state.pane_chrome_item_open;
    let split = state.pane_chrome_split;
    let (_, refs) = context.mount_view_detached(document_id, || {
        let root = entity_ref::<ScrollView>();
        let tabs = entity_ref::<Tabs>();
        let surface_row = entity_ref::<HostStack>();
        let overview: [EntityRef<Card>; 3] = std::array::from_fn(|_| entity_ref());
        let cards: [EntityRef<InteractiveCard>; 3] = std::array::from_fn(|_| entity_ref());
        let tree = entity_ref::<TreeView>();
        let chips: [EntityRef<Chip>; 3] = std::array::from_fn(|_| entity_ref());
        let pane = entity_ref::<PaneChrome>();
        let pane_texts: [EntityRef<Text>; 5] = std::array::from_fn(|_| entity_ref());
        let [pane_tabs, pane_empty, pane_editor, pane_left, pane_right] = pane_texts;
        let pane_tree = entity_ref::<PaneTree>();
        let pane_split = entity_ref::<Button>();
        let pane_close = entity_ref::<IconButton>();
        let labeled = entity_ref::<LabeledValue>();
        let list_items: Vec<EntityRef<ListItem>> =
            LIST_ITEMS.iter().map(|_| entity_ref()).collect();
        let list_leads: Vec<EntityRef<Text>> = LIST_ITEMS.iter().map(|_| entity_ref()).collect();
        let list_labels: Vec<EntityRef<Text>> = LIST_ITEMS.iter().map(|_| entity_ref()).collect();
        let list_trails: Vec<EntityRef<Text>> = LIST_ITEMS.iter().map(|_| entity_ref()).collect();

        let list = widget(HostStack::column(2.0).min_width(LengthSpec::Px(0.0))).with(|list| {
            for (index, (label, meta, disabled)) in LIST_ITEMS.into_iter().enumerate() {
                let selected = state.selected_item == index;
                let item = widget(list_item_spec(label, selected, disabled))
                    .entity_ref(list_items[index])
                    .leading(widget(list_leading_text(selected)).entity_ref(list_leads[index]))
                    .content(widget(list_label_text(label)).entity_ref(list_labels[index]))
                    .trailing(widget(list_trailing_text(meta)).entity_ref(list_trails[index]));
                list.add(if disabled {
                    item
                } else {
                    item.on(queue(pending, move |_: &Activate| {
                        GalleryMessage::SelectListItem(index)
                    }))
                });
            }
        });
        let list_card = showcase("列表", "点按选中一行；禁用的行不响应点按。", list);

        let chip_row = widget(HostStack::leading_row(6.0)).children(
            TREE_SHORTCUTS
                .iter()
                .zip(chips)
                .map(|((id, label), chip)| {
                    let id = *id;
                    widget(Chip::new(*label).selected(state.tree_selected == id))
                        .entity_ref(chip)
                        .on(queue(pending, move |_: &Activate| {
                            GalleryMessage::TreeView(TreeViewEvent::Select(id.to_owned()))
                        }))
                })
                .collect::<Vec<_>>(),
        );
        let tree_card = showcase(
            "层级树",
            "点按箭头展开文件夹，点按行或下方的芯片选中文件。",
            (
                widget(gallery_tree(state))
                    .entity_ref(tree)
                    .on(queue(
                        pending,
                        |event: &TreeViewEvent<Arc<str>>| match event {
                            TreeViewEvent::Toggle(id) => {
                                GalleryMessage::TreeView(TreeViewEvent::Toggle(id.to_string()))
                            }
                            TreeViewEvent::Select(id) => {
                                GalleryMessage::TreeView(TreeViewEvent::Select(id.to_string()))
                            }
                        },
                    ))
                    // one file at a time: any request selects the row it names
                    .on(queue(pending, |event: &TreeSelectionRequested| {
                        GalleryMessage::TreeView(TreeViewEvent::Select(event.id.to_string()))
                    })),
                chip_row,
            ),
        );

        let overview_data = [
            ("基础表面", "主工作区内容层", CardKind::Surface),
            ("抬升表面", "侧栏与工具面板", CardKind::Raised),
            ("选中表面", "当前激活的内容", CardKind::Selected),
        ];
        let overview_views = overview_data
            .into_iter()
            .zip(overview)
            .map(|((title, detail, kind), card)| {
                let mut card_view = Card::new().kind(kind).height(88.0).title(title);
                apply_equal_fill(std::sync::Arc::make_mut(&mut card_view.style.layout), 88.0);
                widget(card_view)
                    .entity_ref(card)
                    .children(widget(styled_text(
                        detail,
                        SemanticColorRole::Muted,
                        type_scale::HINT,
                        type_scale::REGULAR,
                    )))
            })
            .collect::<Vec<_>>();
        let card_views = SURFACE_CARDS
            .into_iter()
            .zip(cards)
            .enumerate()
            .map(|(index, ((title, detail), card))| {
                widget(
                    InteractiveCard::new()
                        .selected(state.selected_surface_card == index)
                        .disabled(index == 2)
                        .style({
                            let mut style = nana_ui::runtime::NodeStyle::default();
                            apply_equal_fill(std::sync::Arc::make_mut(&mut style.layout), 88.0);
                            style
                        }),
                )
                .entity_ref(card)
                .children((
                    widget(styled_text(
                        title,
                        SemanticColorRole::Text,
                        type_scale::BODY,
                        type_scale::REGULAR,
                    )),
                    widget(styled_text(
                        detail,
                        SemanticColorRole::Muted,
                        type_scale::HINT,
                        type_scale::REGULAR,
                    )),
                ))
            })
            .collect::<Vec<_>>();
        // Both sets are built and kept; sync_surfaces reconciles whichever one
        // is visible into surface_row, and the other stays detached.
        let (shown, hidden) = if selected == SurfaceView::Cards {
            (card_views.into_any(), overview_views.into_any())
        } else {
            (overview_views.into_any(), card_views.into_any())
        };
        let surfaces_card =
            showcase(
                "表面层级",
                "「概览」是三种表面；切到「卡片」可以点选交互卡片，禁用的卡片不可选。",
                (
                    widget(HostStack::fill_row(12.0).align(nana_ui::runtime::AlignSpec::Center))
                        .children((
                            widget(
                                Tabs::new(if selected == SurfaceView::Cards {
                                    "cards"
                                } else {
                                    "overview"
                                })
                                .options([
                                    TabOption::new("overview", "概览"),
                                    TabOption::new("cards", "卡片"),
                                ]),
                            )
                            .entity_ref(tabs)
                            .on(queue(pending, |event: &TabsEvent| match event {
                                TabsEvent::Select(value) if value.as_ref() == "cards" => {
                                    GalleryMessage::SelectSurfaceView(SurfaceView::Cards)
                                }
                                TabsEvent::Select(_) => {
                                    GalleryMessage::SelectSurfaceView(SurfaceView::Overview)
                                }
                                _ => GalleryMessage::OverlayInteraction,
                            })),
                            widget(HostStack::spacer()),
                            widget(
                                HostStack::column(0.0)
                                    .width(LengthSpec::Px(200.0))
                                    .grow(0.0)
                                    .shrink(0.0),
                            )
                            .children(widget(selected_card_value(state)).entity_ref(labeled)),
                        )),
                    // The three surfaces sit on the page colour, as they would in
                    // an application, not on this card's own surface.
                    widget(
                        HostStack::column(0.0)
                            .padding(12.0)
                            .background(SemanticColorRole::Background),
                    )
                    .children(
                        widget(HostStack::fill_row(12.0))
                            .entity_ref(surface_row)
                            .children(shown),
                    ),
                ),
            );

        let media_card = showcase(
            "媒体与头像",
            "缩略图的空、加载、就绪与不可用四种状态，以及头像。",
            (
                widget(HostStack::leading_row(10.0)).children((
                    widget(Thumbnail::empty()),
                    widget(Thumbnail::loading()),
                    widget(Thumbnail::new("gallery.thumb")),
                    widget(Thumbnail::unavailable()),
                )),
                widget(HostStack::leading_row(10.0)).children((
                    widget(Avatar::new("gallery.thumb").label("预览")),
                    widget(Avatar::new("gallery.thumb").size(40.0).label("大号预览")),
                    widget(Avatar::empty().label("未设置")),
                )),
            ),
        );
        let empty_card = showcase(
            "空状态",
            "区域里还没有内容时给出的说明。",
            widget(EmptyState::new("没有打开的文件").message("从层级树里选择一个文件")),
        );

        let pane_text = |value: &'static str, color, text: EntityRef<Text>| {
            widget(hugging_text(
                value,
                color,
                type_scale::HINT,
                type_scale::REGULAR,
            ))
            .entity_ref(text)
        };
        let split_view = widget(
            Button::new("左右分栏")
                .kind(ButtonKind::Text)
                .size(ControlSize::Small),
        )
        .entity_ref(pane_split)
        .on(queue(pending, |_: &Activate| {
            GalleryMessage::PaneChrome(PaneChromeActionKind::SplitHorizontal)
        }));
        let close_view = widget(
            IconButton::new(Icon::Close, "关闭 Item")
                .size(ControlSize::Small)
                .kind(ButtonKind::Text),
        )
        .entity_ref(pane_close)
        .on(queue(pending, |_: &Activate| {
            GalleryMessage::PaneChrome(PaneChromeActionKind::CloseItem)
        }));
        // The pane tree's leaves are detached; `reconcile_children` below
        // places the ones its layout shows. They are built first, so the
        // tree's root can name them.
        let pane_tree_view = (
            detached(pane_text(
                "Item 已关闭",
                SemanticColorRole::Muted,
                pane_empty,
            )),
            detached(pane_text(
                "编辑器内容",
                SemanticColorRole::Text,
                pane_editor,
            )),
            detached(pane_text("左侧编辑器", SemanticColorRole::Text, pane_left)),
            detached(pane_text("右侧编辑器", SemanticColorRole::Text, pane_right)),
            widget(PaneTree::new(PaneTreeNode::leaf("empty")))
                .entity_ref(pane_tree)
                .bind(move |tree: &mut PaneTree| {
                    let built = "the pane tree's leaves are built before it";
                    tree.root = pane_tree_node_for(
                        item_open,
                        split,
                        pane_empty.get().expect(built),
                        pane_editor.get().expect(built),
                        pane_left.get().expect(built),
                        pane_right.get().expect(built),
                    );
                }),
        );
        // Both actions are the pane's; which ones its header shows is
        // `actions`, set from the state once mounted and on every sync.
        let pane_view = widget(PaneChrome::new())
            .entity_ref(pane)
            .tabs(pane_text(
                if item_open { "main.rs" } else { "空窗格" },
                SemanticColorRole::Text,
                pane_tabs,
            ))
            .action(
                PaneChromeAction::new(PaneChromeActionKind::SplitHorizontal, "左右分栏"),
                split_view,
            )
            .action(
                PaneChromeAction::new(PaneChromeActionKind::CloseItem, "关闭 Item")
                    .icon(Icon::Close),
                close_view,
            )
            .body(pane_tree_view);
        let pane_card = showcase(
            "窗格",
            "窗格标题栏只显示当前可用的操作：分栏把内容分成左右两半，关闭后窗格变空。",
            widget(
                HostStack::column(0.0)
                    .height(LengthSpec::Px(150.0))
                    .background(SemanticColorRole::Background),
            )
            .children(pane_view),
        );

        let content =
            widget(page_scroll())
                .entity_ref(root)
                .children(widget(page_body()).children((
                    page_header(
                        "展示与容器",
                        "列表、树、卡片表面、媒体与窗格，选择状态都由应用保存。",
                    ),
                    card_row((list_card, tree_card)),
                    card_row(surfaces_card),
                    card_row((pane_card, empty_card)),
                    card_row(media_card),
                )));
        let view = (content, hidden);
        with_refs(
            view,
            (
                (root, tabs, surface_row, overview, cards, tree),
                (chips, pane, pane_texts),
                (pane_tree, pane_split, pane_close, labeled),
                (list_items, list_leads, list_labels, list_trails),
            ),
        )
    })?;
    let (
        (root, tabs, surface_row, overview, cards, tree),
        (chips, pane, [pane_tabs, pane_empty, pane_editor, pane_left, pane_right]),
        (pane_tree, pane_split, pane_close, labeled),
        (list_items, list_leads, list_labels, list_trails),
    ) = refs;
    reconcile_children(
        context,
        pane_tree.stable_id(),
        &pane_tree_children(state, pane_empty, pane_editor, pane_left, pane_right),
    )?;
    context.update_component(pane, |chrome, _| {
        chrome.actions = pane_actions(state, pane_split.stable_id(), pane_close.stable_id());
    })?;
    context.assemble_pane_chrome(pane)?;
    Ok(SurfacesTree {
        root,
        tabs,
        surface_row,
        overview,
        cards,
        tree,
        chips,
        pane,
        pane_tabs,
        pane_tree,
        pane_empty,
        pane_editor,
        pane_left,
        pane_right,
        pane_split,
        pane_close,
        labeled,
        list_items,
        list_leads,
        list_labels,
        list_trails,
    })
}

fn mount_feedback(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    state: &GalleryState,
    pending: &Arc<Mutex<Vec<GalleryMessage>>>,
) -> Result<FeedbackTree, FrameworkError> {
    let (_, refs) = context.mount_view_detached(document_id, || {
        let root = entity_ref::<ScrollView>();
        let progress = entity_ref::<Progress>();
        let spinner = entity_ref::<Spinner>();
        let skeleton = entity_ref::<Skeleton>();
        let badge = entity_ref::<StatusBadge>();
        let validation = entity_ref::<ValidationMessage>();
        let toast = entity_ref::<Toast>();
        let actions: [EntityRef<Button>; 5] = std::array::from_fn(|_| entity_ref());
        let [start, dialog, context_button, image, popover_action] = actions;
        let popover = entity_ref::<Popover>();
        let progress_label = entity_ref::<Text>();

        let progress_card = showcase(
            "进度与加载",
            "开始一个约 1 秒的任务：按钮进入加载状态，进度条前进，完成后恢复。",
            (
                widget(HostStack::leading_row(12.0)).children((
                    widget(start_loading_button(state))
                        .entity_ref(start)
                        .on(queue(pending, |_: &Activate| GalleryMessage::ToggleLoading)),
                    widget(progress_spinner(state)).entity_ref(spinner),
                )),
                widget(Progress::new(progress_value(state), 100.0)).entity_ref(progress),
                widget(progress_label_text(state)).entity_ref(progress_label),
                widget(progress_skeleton(state)).entity_ref(skeleton),
            ),
        );
        let status_card = showcase(
            "状态与通知",
            "最近一次操作同步到状态徽章与通知条；删除需要在菜单里再确认一次。",
            (
                widget(HostStack::leading_row(0.0)).children(
                    widget(StatusBadge::new(action_status(state), StatusTone::Info))
                        .entity_ref(badge),
                ),
                widget(ValidationMessage::new(
                    "删除会清空列表选择，需要再次确认",
                    ValidationIntent::Warning,
                ))
                .entity_ref(validation),
                widget(Toast::new(action_status(state), ToastTone::Info)).entity_ref(toast),
            ),
        );

        let popover_view = widget(
            Popover::new()
                .trigger("查看当前状态")
                .open(state.popover_open),
        )
        .entity_ref(popover)
        .on(queue(pending, |event: &PopoverToggled| {
            if event.open {
                GalleryMessage::TogglePopover
            } else {
                GalleryMessage::ClosePopover
            }
        }))
        .on(queue(pending, |_: &PopoverClosed| {
            GalleryMessage::ClosePopover
        }))
        .children(
            widget(popover_action_button(state.popover_open))
                .entity_ref(popover_action)
                .on(queue(pending, |_: &Activate| GalleryMessage::PrimaryAction)),
        );
        let overlay_card = showcase(
            "浮层",
            "对话框、上下文菜单（也可以右键「更多操作」）、图片查看器与弹出层。",
            widget(HostStack::leading_row(8.0)).children((
                widget(dialog_button(state))
                    .entity_ref(dialog)
                    .on(queue(pending, |_: &Activate| GalleryMessage::ToggleDialog)),
                widget(Button::new("更多操作").kind(ButtonKind::Subtle))
                    .entity_ref(context_button)
                    .on(queue(pending, |_: &Activate| {
                        GalleryMessage::ToggleContextMenu
                    })),
                widget(Button::new("查看图片").kind(ButtonKind::Subtle))
                    .entity_ref(image)
                    .on(queue(pending, |_: &Activate| {
                        GalleryMessage::ToggleImageViewer
                    })),
                popover_view,
            )),
        );

        let view = widget(page_scroll())
            .entity_ref(root)
            .children(widget(page_body()).children((
                page_header("反馈与浮层", "进度、状态与通知，以及对话框、菜单等浮层。"),
                card_row((progress_card, status_card)),
                card_row(overlay_card),
            )));
        with_refs(
            view,
            (
                (root, progress, spinner, skeleton, progress_label),
                (badge, validation, toast),
                actions,
                popover,
            ),
        )
    })?;
    let (
        (root, progress, spinner, skeleton, progress_label),
        (badge, validation, toast),
        [start, dialog, context_button, image, popover_action],
        popover,
    ) = refs;
    Ok(FeedbackTree {
        root,
        progress,
        progress_label,
        spinner,
        skeleton,
        badge,
        _validation: validation,
        toast,
        start,
        dialog,
        context: context_button,
        _image: image,
        popover,
        popover_action,
    })
}

fn mount_data(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    state: &GalleryState,
    pending: &Arc<Mutex<Vec<GalleryMessage>>>,
) -> Result<DataTree, FrameworkError> {
    let (_, refs) = context.mount_view_detached(document_id, || {
        let root = entity_ref::<ScrollView>();
        let trend = entity_ref::<Chart>();
        let share = entity_ref::<Chart>();
        let calendar = entity_ref::<CalendarHeatmap>();
        let texts: [EntityRef<Text>; 3] = std::array::from_fn(|_| entity_ref());
        let [trend_status, share_status, calendar_status] = texts;

        let trend_card = showcase(
            "趋势",
            "柱状与折线共用类目轴；点按一根柱或一个点查看数值，点按图例切换系列。",
            (
                widget(
                    Chart::new(trend_chart_option())
                        .label("本周访问与转化，周二最高")
                        .style(chart_style()),
                )
                .entity_ref(trend)
                .on(queue(pending, |event: &ChartEvent| {
                    chart_message(GalleryChart::Trend, event)
                })),
                widget(chart_status_text(state, GalleryChart::Trend)).entity_ref(trend_status),
            ),
        );
        let share_card = showcase(
            "构成",
            "环形图按扇区强调；点按扇区查看占比，点按图例隐藏扇区。",
            (
                widget(
                    Chart::new(share_chart_option())
                        .label("资源构成，文本占比最高")
                        .style(chart_style()),
                )
                .entity_ref(share)
                .on(queue(pending, |event: &ChartEvent| {
                    chart_message(GalleryChart::Share, event)
                })),
                widget(chart_status_text(state, GalleryChart::Share)).entity_ref(share_status),
            ),
        );
        let calendar_card = showcase(
            "日历热力图",
            "按日期着色的三个月活动；移动指针查看当天的数值。",
            (
                widget(CalendarHeatmap::new(gallery_calendar_data())).entity_ref(calendar),
                widget(calendar_status_text(state)).entity_ref(calendar_status),
            ),
        );
        let view = widget(page_scroll())
            .entity_ref(root)
            .children(widget(page_body()).children((
                page_header("数据可视化", "图表与日历热力图，数据都由应用提供。"),
                card_row((trend_card, share_card)),
                card_row(calendar_card),
            )));
        with_refs(view, (root, trend, share, calendar, texts))
    })?;
    let (root, _trend, _share, calendar, [trend_status, share_status, calendar_status]) = refs;
    Ok(DataTree {
        root,
        calendar,
        calendar_status,
        trend_status,
        share_status,
    })
}

fn mount_rich_text(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    state: &GalleryState,
    pending: &Arc<Mutex<Vec<GalleryMessage>>>,
) -> Result<RichTextMount, FrameworkError> {
    let (_, refs) = context.mount_view_detached(document_id, || {
        let root = entity_ref::<ScrollView>();
        let drop = entity_ref::<Card>();
        let markdown = entity_ref::<NativeMarkdown>();
        let texts: [EntityRef<Text>; 2] = std::array::from_fn(|_| entity_ref());
        let [link_status, drop_hint] = texts;
        let editor = entity_ref::<TextArea>();
        let terminal = entity_ref::<TerminalView>();
        let diff = entity_ref::<DiffView>();

        // #59: `vertical-rl` in columns — upright CJK with its vertical
        // punctuation forms, sideways Latin and digits, wrapped against the
        // box height and stacked from the right.
        let vertical = {
            let mut text = hugging_text(
                "縦書き「春はあけぼの」。やうやう白くなりゆく山ぎは、NanaUI 2026年、少しあかりて。",
                SemanticColorRole::Text,
                16.0,
                400,
            );
            let layout = Arc::make_mut(&mut text.style.layout);
            layout.writing_mode = Some(nana_ui::runtime::WritingModeSpec::VerticalRl);
            layout.height = Some(LengthSpec::Px(180.0));
            text
        };
        // The same writing mode on an editor: its value, caret and
        // selection are laid out and hit in the same columns.
        let vertical_editor = {
            let mut area =
                TextArea::new("竖排编辑：「光标」沿列移动。\nABC 与 123 侧卧。").height(180.0);
            let layout = Arc::make_mut(&mut area.style.layout);
            layout.writing_mode = Some(nana_ui::runtime::WritingModeSpec::VerticalRl);
            layout.width = Some(LengthSpec::Px(160.0));
            area
        };
        // `direction: rtl` in a vertical mode starts the line at the
        // bottom; CJK still reads down the column.
        let vertical_rtl = {
            let mut text = hugging_text("行首在底端", SemanticColorRole::Muted, 16.0, 400);
            let layout = Arc::make_mut(&mut text.style.layout);
            layout.writing_mode = Some(nana_ui::runtime::WritingModeSpec::VerticalRl);
            layout.dir = Some(nana_ui::runtime::DirSpec::Rtl);
            layout.height = Some(LengthSpec::Px(180.0));
            text
        };

        let markdown_card = showcase(
            "Markdown",
            "CommonMark、公式与 Mermaid 流程图在同一棵树里排版；点按链接会在下方显示地址。",
            (
                widget(state.markdown.clone()).entity_ref(markdown),
                widget(link_status_text(state)).entity_ref(link_status),
            ),
        );
        let editor_card = showcase(
            "代码编辑器",
            "行号、小地图、诊断与 Git 标记；与「多行文本」共用内容和可编辑状态。",
            widget(gallery_code_editor(state)).entity_ref(editor),
        );
        let terminal_card = showcase(
            "终端",
            "屏幕内容由应用逐格提供。",
            widget(gallery_terminal_view()).entity_ref(terminal),
        );
        let diff_card = showcase(
            "差异",
            "按块显示增删的行，每块带接受与拒绝操作。",
            widget(gallery_diff_view()).entity_ref(diff),
        );
        let drop_card = showcase(
            "拖入文件",
            "把文件拖到下面的区域，放下后显示文件名。",
            // A card, not a plain stack: a stack never takes a hit, so a drag
            // over it would land on the showcase card around it instead.
            widget(drop_zone())
                .entity_ref(drop)
                .on({
                    let pending = Arc::clone(pending);
                    move |event: &FileDropEvent| {
                        let FileDropEvent::Dropped { paths, .. } = event else {
                            return;
                        };
                        let message = GalleryMessage::FilesDropped(paths.to_vec());
                        if let Ok(mut queue) = pending.lock() {
                            queue.push(message);
                        }
                    }
                })
                .children(widget(gallery_drop_hint(state)).entity_ref(drop_hint)),
        );
        let vertical_card = showcase(
            "竖排文字",
            "竖排正文、竖排编辑框，以及从底端起排的一行。",
            widget(HostStack::leading_row(32.0)).children((
                widget(vertical),
                widget(vertical_editor),
                widget(vertical_rtl),
            )),
        );
        let view = widget(page_scroll()).entity_ref(root).children(
            widget(page_body()).children((
                page_header(
                    "富文本",
                    "Markdown、代码、终端、差异与竖排文字，共用同一个文本引擎。",
                ),
                // The drop zone stays above the fold: file drags are matched
                // against unscrolled layout boxes, so a scrolled-down zone
                // would miss them.
                card_row((
                    markdown_card,
                    widget(
                        HostStack::column(16.0)
                            .width(LengthSpec::Px(360.0))
                            .grow(0.0)
                            .shrink(0.0),
                    )
                    .children((card_row(drop_card), card_row(vertical_card))),
                )),
                card_row((editor_card, terminal_card)),
                card_row(diff_card),
            )),
        );
        with_refs(
            view,
            (root, drop, markdown, texts, editor, (terminal, diff)),
        )
    })?;
    let (root, drop, markdown, [link_status, drop_hint], editor, (terminal, diff)) = refs;
    context.set_drop_target(drop, DropAccepts::files())?;
    context.refresh_terminal_view(terminal)?;
    Ok((
        root,
        markdown,
        link_status,
        drop,
        drop_hint,
        editor,
        terminal,
        diff,
    ))
}

fn gallery_drop_hint(state: &GalleryState) -> nana_ui::runtime::Text {
    if state.dropped_paths.is_empty() {
        return styled_text(
            "把文件拖到这里",
            SemanticColorRole::Muted,
            type_scale::META,
            type_scale::REGULAR,
        );
    }
    let names = state
        .dropped_paths
        .iter()
        .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
        .collect::<Vec<_>>()
        .join("、");
    styled_text(
        format!("已放入：{names}"),
        SemanticColorRole::Accent,
        12.0,
        400,
    )
}

fn gallery_terminal_screen() -> TerminalScreen {
    let mut screen = TerminalScreen::blank(32, 6);
    let hello = "nana@host ~ % echo hi";
    let cells = Arc::make_mut(&mut screen.cells);
    for (index, ch) in hello.chars().enumerate() {
        if let Some(cell) = cells.get_mut(index) {
            cell.text = Arc::from(ch.to_string());
        }
    }
    screen
}

fn gallery_diff_hunks() -> Arc<[DiffHunk]> {
    Arc::from([DiffHunk::new(
        "@@ -1,2 +1,2 @@",
        vec![
            DiffLine::context(1, 1, "fn main() {"),
            DiffLine::removed(2, "    println!(\"a\");"),
            DiffLine::added(2, "    println!(\"b\");"),
        ],
    )])
}

fn mount_graph(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    state: &GalleryState,
    pending: &Arc<Mutex<Vec<GalleryMessage>>>,
) -> Result<GraphMount, FrameworkError> {
    let (_, refs) = context.mount_view_detached(document_id, || {
        let root = entity_ref::<HostStack>();
        let graph = entity_ref::<GraphCanvas>();
        let minimap = entity_ref::<GraphMinimap>();
        let selection = entity_ref::<Text>();
        let reset = entity_ref::<Button>();
        let view = widget(HostStack::canvas().padding_xy(20.0, 18.0))
            .entity_ref(root)
            .children((
                widget(HostStack::fill_row(12.0).align(nana_ui::runtime::AlignSpec::Center))
                    .children((
                        page_header(
                            "节点图",
                            "拖动节点、连线端口，滚轮缩放；右下角的小地图可以拖动视口。",
                        ),
                        widget(hugging_text(
                            graph_selection_label(state),
                            SemanticColorRole::Muted,
                            type_scale::HINT,
                            type_scale::REGULAR,
                        ))
                        .entity_ref(selection),
                        widget(
                            Button::new("重置视图")
                                .kind(ButtonKind::Text)
                                .size(ControlSize::Small),
                        )
                        .entity_ref(reset)
                        .on(queue(pending, |_: &Activate| {
                            GalleryMessage::ResetGraphViewport
                        })),
                    )),
                widget(
                    GraphCanvas::new("gallery", state.graph.clone())
                        .viewport(state.graph_viewport)
                        .selection(state.graph_selection.clone()),
                )
                .entity_ref(graph)
                .on(queue(pending, |event: &GraphCanvasEvent| {
                    GalleryMessage::Graph(event.clone())
                })),
                widget(
                    GraphMinimap::new(state.graph.clone())
                        .canvas_size(GraphSize::new(900.0, 560.0))
                        .viewport(state.graph_viewport)
                        .style(graph_minimap_style()),
                )
                .entity_ref(minimap)
                .on(queue(pending, |event: &GraphMinimapEvent| {
                    GalleryMessage::GraphMinimap(event.clone())
                })),
            ));
        with_refs(view, (root, graph, minimap, selection, reset))
    })?;
    Ok(refs)
}

fn graph_minimap_style() -> NodeStyle {
    NodeStyle {
        layout: Arc::new(nana_ui::runtime::LayoutStyle {
            position: PositionSpec::Absolute,
            offset_right: Some(LengthSpec::Px(12.0)),
            offset_bottom: Some(LengthSpec::Px(12.0)),
            width: Some(LengthSpec::Px(180.0)),
            height: Some(LengthSpec::Px(135.0)),
            z_index: Some(3),
            ..nana_ui::runtime::LayoutStyle::default()
        }),
        ..NodeStyle::default()
    }
}

fn mount_workspace(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    state: &GalleryState,
    pending: &Arc<Mutex<Vec<GalleryMessage>>>,
) -> Result<WorkspaceTree, FrameworkError> {
    let (_, refs) = context.mount_view_detached(document_id, || {
        let root = entity_ref::<HostStack>();
        let dock = entity_ref::<nana_ui::runtime::Dock>();
        let panels: Vec<EntityRef<HostStack>> = DOCK_PANELS.iter().map(|_| entity_ref()).collect();
        let buttons: [EntityRef<Button>; 3] = std::array::from_fn(|_| entity_ref());
        let [lock, hide, reset] = buttons;
        let status = entity_ref::<Text>();

        let locked = state.dock_locked;
        let hidden_assets = !state.dock_is_visible("gallery.assets");
        let tool = |label: &'static str, button: EntityRef<Button>| {
            widget(
                Button::new(label)
                    .kind(ButtonKind::Subtle)
                    .size(ControlSize::Small),
            )
            .entity_ref(button)
        };
        let tools = widget(
            HostStack::fill_row(8.0)
                .align(nana_ui::runtime::AlignSpec::Center)
                .padding_xy(12.0, 8.0)
                .background(SemanticColorRole::Surface)
                .grow(0.0)
                .shrink(0.0),
        )
        .children((
            tool(if locked { "解锁 Dock" } else { "锁定 Dock" }, lock),
            tool(
                if hidden_assets {
                    "恢复 Assets"
                } else {
                    "隐藏 Assets"
                },
                hide,
            ),
            tool("重置 Dock", reset).on(queue(pending, |_: &Activate| {
                GalleryMessage::Dock(GalleryDock::Reset)
            })),
            widget(workspace_status_text(dock_status(state))).entity_ref(status),
        ));
        // Each panel is keyed with its dock item id: the dock binds it as
        // that item's content.
        let panel_views = DOCK_PANELS
            .iter()
            .zip(&panels)
            .map(|((id, title, hint), panel)| dock_panel(id, title, hint, *panel))
            .collect::<Vec<_>>();
        // The dock takes the rest of the primary region: its height is
        // definite there, so growing into it is safe.
        let dock_frame = widget(
            HostStack::fill_column(0.0).min_height(LengthSpec::Px(WORKSPACE_DOCK_MIN_HEIGHT)),
        )
        .children(
            widget(runtime_dock_from_workspace(
                state,
                &std::collections::HashMap::new(),
            ))
            .entity_ref(dock)
            .children(panel_views),
        );
        let view = widget(
            HostStack::fill_column(WORKSPACE_CANVAS_GAP)
                .padding(WORKSPACE_CANVAS_PADDING)
                .background(SemanticColorRole::Background)
                .grow(0.0),
        )
        .entity_ref(root)
        .children((tools, dock_frame));
        with_refs(view, (root, dock, panels, buttons, status))
    })?;
    let (root, dock, panels, [lock, hide, reset], status) = refs;
    Ok(WorkspaceTree {
        root,
        dock,
        panels: DOCK_PANELS
            .iter()
            .map(|(id, _, _)| (*id).to_owned())
            .zip(panels)
            .collect(),
        lock,
        hide,
        _reset: reset,
        status,
    })
}

/// One dock item's content, keyed with the item's id.
fn dock_panel(
    id: &str,
    title: &str,
    hint: &str,
    panel: EntityRef<HostStack>,
) -> impl IntoView + use<> {
    widget(HostStack::fill_column(5.0).padding(10.0))
        .key(id.to_owned())
        .entity_ref(panel)
        .children((
            widget(styled_text(
                title,
                SemanticColorRole::Text,
                type_scale::META,
                type_scale::REGULAR,
            )),
            widget(styled_text(
                hint,
                SemanticColorRole::Muted,
                type_scale::HINT,
                type_scale::REGULAR,
            )),
        ))
}

/// A region's slot around its content: a column headed by `title` and a
/// button that collapses region `collapse`, then `body`.
#[allow(clippy::too_many_arguments)]
fn region_panel<B: IntoView>(
    pending: &Arc<Mutex<Vec<GalleryMessage>>>,
    (gap, padding_y): (f32, f32),
    title: &'static str,
    collapse: RegionId,
    [slot, root]: [EntityRef<HostStack>; 2],
    button: EntityRef<Button>,
    body: B,
) -> impl IntoView + use<B> {
    let heading = widget(
        HostStack::fill_row(8.0)
            .align(nana_ui::runtime::AlignSpec::Center)
            .grow(0.0),
    )
    .children((
        widget(hugging_text(title, SemanticColorRole::Muted, 12.0, 700)),
        widget(HostStack::spacer()),
        widget(
            Button::new("收起")
                .kind(ButtonKind::Text)
                .size(ControlSize::Small),
        )
        .entity_ref(button)
        .on(queue(pending, move |_: &Activate| {
            GalleryMessage::Workspace(WorkspaceAction::ToggleRegion(collapse.clone()))
        })),
    ));
    widget(HostStack::region_slot()).entity_ref(slot).children(
        widget(
            HostStack::fill_column(gap)
                .padding_xy(12.0, padding_y)
                .grow(0.0),
        )
        .entity_ref(root)
        .children((heading, body)),
    )
}

fn mount_inspector(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    state: &GalleryState,
    pending: &Arc<Mutex<Vec<GalleryMessage>>>,
) -> Result<InspectorTree, FrameworkError> {
    let radius = state.appearance.standard_radius().round() as u8;
    let (_, refs) = context.mount_view_detached(document_id, || {
        let stacks: [EntityRef<HostStack>; 2] = std::array::from_fn(|_| entity_ref());
        let collapse = entity_ref::<Button>();
        let slider = entity_ref::<nana_ui::runtime::RangeField>();
        let corners = entity_ref::<Switch>();
        let body = (
            widget(styled_text(
                "调整后立即作用于整个界面。",
                SemanticColorRole::Muted,
                type_scale::HINT,
                type_scale::REGULAR,
            )),
            widget(
                nana_ui::runtime::RangeField::new(f64::from(radius), 0.0, 24.0, 1.0)
                    .label("标准圆角")
                    .unit("px"),
            )
            .entity_ref(slider)
            .on(queue(pending, |event: &RangeInput| {
                GalleryMessage::SetStandardRadius(event.value.round() as u8)
            })),
            widget(Switch::new(
                "主区域圆角",
                state.appearance.workspace_corners_enabled(),
            ))
            .entity_ref(corners)
            .on(queue(pending, |event: &ToggleChanged| {
                GalleryMessage::SetWorkspaceCorners(event.checked)
            })),
        );
        let view = region_panel(
            pending,
            (10.0, 10.0),
            "检查器",
            RegionId::Inspector,
            stacks,
            collapse,
            body,
        );
        with_refs(view, (stacks, collapse, slider, corners))
    })?;
    let ([slot, root], collapse, radius, corners) = refs;
    Ok(InspectorTree {
        slot,
        _root: root,
        _collapse: collapse,
        radius,
        corners,
    })
}

fn mount_bottom(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    pending: &Arc<Mutex<Vec<GalleryMessage>>>,
) -> Result<(Entity<HostStack>, Entity<Button>), FrameworkError> {
    let (_, ([slot, _], collapse)) = context.mount_view_detached(document_id, || {
        let stacks: [EntityRef<HostStack>; 2] = std::array::from_fn(|_| entity_ref());
        let collapse = entity_ref::<Button>();
        // A shell with no sidebar or dock, framed as the window it would be.
        let popup = widget(
            HostStack::fill_column(0.0)
                .width(LengthSpec::Px(WORKSPACE_POPUP_WIDTH))
                .background(SemanticColorRole::Background)
                .grow(0.0)
                .shrink(0.0),
        )
        .children(
            widget(AppShell::new())
                .title_bar(widget(AppTitleBar::new("独立窗口")))
                .body(
                    widget(HostStack::column(4.0).padding_xy(12.0, 8.0)).children((
                        widget(styled_text(
                            "AppShell",
                            SemanticColorRole::Text,
                            type_scale::META,
                            type_scale::MEDIUM,
                        )),
                        widget(styled_text(
                            "只有标题栏和内容的壳，没有侧栏与 Dock。",
                            SemanticColorRole::Muted,
                            type_scale::HINT,
                            type_scale::REGULAR,
                        )),
                    )),
                ),
        );
        let status = widget(HostStack::column(8.0).min_width(LengthSpec::Px(0.0))).children((
            widget(HostStack::leading_row(0.0))
                .children(widget(StatusBadge::new("布局就绪", StatusTone::Success))),
            widget(styled_text(
                "工具栏、检查器和这块底部面板都是工作区区域：拖动边界调整尺寸，收起后再展开会回到原来的大小。",
                SemanticColorRole::Muted,
                type_scale::HINT,
                type_scale::REGULAR,
            )),
        ));
        let view = region_panel(
            pending,
            (8.0, 8.0),
            "底部面板",
            RegionId::Diagnostics,
            stacks,
            collapse,
            widget(HostStack::fill_row(16.0).height(LengthSpec::Fill).min_height(LengthSpec::Px(0.0)).grow(1.0))
                .children((popup, status)),
        );
        with_refs(view, (stacks, collapse))
    })?;
    Ok((slot, collapse))
}

fn mount_toolbar(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    pending: &Arc<Mutex<Vec<GalleryMessage>>>,
) -> Result<(Entity<HostStack>, Entity<Button>), FrameworkError> {
    let (_, refs) = context.mount_view_detached(document_id, || {
        let slot = entity_ref::<HostStack>();
        let reset = entity_ref::<Button>();
        let view = widget(HostStack::region_slot()).entity_ref(slot).children(
            widget(
                HostStack::fill_row(8.0)
                    .align(nana_ui::runtime::AlignSpec::Center)
                    .height(LengthSpec::Fill)
                    .padding_xy(10.0, 0.0)
                    .grow(0.0),
            )
            .children((
                widget(hugging_text("工作区", SemanticColorRole::Text, 13.0, 700)),
                widget(HostStack::spacer()),
                widget(
                    Button::new("恢复默认")
                        .kind(ButtonKind::Text)
                        .size(ControlSize::Small),
                )
                .entity_ref(reset)
                .on(queue(pending, |_: &Activate| {
                    GalleryMessage::ResetWorkspaceLayout
                })),
            )),
        );
        with_refs(view, (slot, reset))
    })?;
    Ok(refs)
}

fn sync_controls(
    context: &mut nana_ui::runtime::AppContext,
    tree: &ControlsTree,
    state: &GalleryState,
) {
    let _ = context.update_component(tree.loading, |button, _| {
        *button = loading_button(state);
    });
    let _ = context.update_component(tree.clicks, |label, _| {
        *label = click_count_text(state);
    });
    for (index, control) in tree.segmented.iter().copied().enumerate() {
        let selected = if state.checked {
            tree.segmented_on[index]
        } else {
            tree.segmented_off[index]
        };
        let _ = context.set_segmented_options(
            control,
            vec![tree.segmented_off[index], tree.segmented_on[index]],
            Some(selected),
        );
    }
    for input in tree.inputs {
        let _ = context.update_component(input, |field, _| {
            field.state = nana_ui::runtime::TextInputState::new(state.input.clone());
            field.invalid = state.input.trim().is_empty();
        });
    }
    let _ = context.update_component(tree.secure, |field, _| {
        field.state = nana_ui::runtime::TextInputState::new(state.input.clone());
    });
    for (index, dropdown) in tree.dropdowns.iter().copied().enumerate() {
        let placeholder = ["小号", "中号", "大号"][index];
        // `set_component`, not an assignment: the menu stays open across a
        // refresh that did not change the selection.
        let size = context
            .read(dropdown, |field| field.size)
            .unwrap_or_default();
        let _ = context.set_component(dropdown, gallery_dropdown(state, placeholder, size));
    }
    let _ = context.update_component(tree.field_status, |label, _| {
        *label = field_status_text(state);
    });
    let _ = context.update_component(tree.selection_status, |label, _| {
        *label = selection_status_text(state);
    });
    let _ = context.set_component(tree.checkbox, Checkbox::new("启用选项", state.checked));
    let _ = context.update_component(tree.switch, |switch, _| {
        *switch = Switch::new("允许编辑说明", state.switched).disabled(!state.checked);
    });
    let _ = context.update_component(tree.range, |range, _| {
        range.value = f64::from(state.slider);
    });
    let _ = context.update_component(tree.meter, |meter, _| {
        meter.value = f32::from(state.slider) / 100.0;
    });
    let _ = context.set_component(tree.search, gallery_search(state));
    let _ = context.update_component(tree.textarea, |area, _| {
        *area = gallery_textarea(state);
    });
    let _ = context.update_component(tree.editor_status, |label, _| {
        *label = editor_status_text(state);
    });
    let _ = context.update_component(tree.xy_pad, |pad, _| {
        pad.value = state.xy_pad;
    });
    let _ = context.update_component(tree.xy_label, |label, _| {
        *label = xy_label_text(state);
    });
}

fn sync_surfaces(
    context: &mut nana_ui::runtime::AppContext,
    tree: &SurfacesTree,
    state: &GalleryState,
) {
    for (index, item) in tree.list_items.iter().copied().enumerate() {
        let (label, meta, disabled) = LIST_ITEMS[index];
        let selected = state.selected_item == index;
        let leading = tree.list_leads[index];
        let content = tree.list_labels[index];
        let trailing = tree.list_trails[index];
        let _ = context.update_component(leading, |mark, _| {
            *mark = list_leading_text(selected);
        });
        let _ = context.update_component(content, |mark, _| {
            *mark = list_label_text(label);
        });
        let _ = context.update_component(trailing, |mark, _| {
            *mark = list_trailing_text(meta);
        });
        let _ = context.update_component(item, |row, _| {
            *row = gallery_list_item(label, selected, disabled, leading, content, trailing);
        });
        let _ = context.set_list_item_slots(item, list_item_slots(leading, content, trailing));
    }
    let selected = SurfaceView::from_index(state.surface_selection.selected());
    let _ = context.update_component(tree.tabs, |tabs, _| {
        tabs.selected = Some(Arc::from(if selected == SurfaceView::Cards {
            "cards"
        } else {
            "overview"
        }));
    });
    let visible: Vec<StableNodeId> = if selected == SurfaceView::Cards {
        tree.cards.iter().map(|entity| entity.stable_id()).collect()
    } else {
        tree.overview
            .iter()
            .map(|entity| entity.stable_id())
            .collect()
    };
    let _ = reconcile_children(context, tree.surface_row.stable_id(), &visible);
    for (index, card) in tree.cards.iter().copied().enumerate() {
        let _ = context.update_component(card, |card, _| {
            card.selected = state.selected_surface_card == index;
            card.disabled = index == 2;
        });
    }
    let _ = context.update_component(tree.tree, |view, _| {
        *view = gallery_tree(state);
    });
    for (index, chip) in tree.chips.iter().copied().enumerate() {
        let selected = state.tree_selected == TREE_SHORTCUTS[index].0;
        let _ = context.update_component(chip, |chip, _| {
            chip.selected = selected;
        });
    }
    let _ = context.update_component(tree.pane_tabs, |label, _| {
        *label = hugging_text(
            if state.pane_chrome_item_open {
                "main.rs"
            } else {
                "空窗格"
            },
            SemanticColorRole::Text,
            type_scale::HINT,
            type_scale::REGULAR,
        );
    });
    let _ = context.update_component(tree.pane_tree, |pane, _| {
        pane.root = pane_tree_node(
            state,
            tree.pane_empty,
            tree.pane_editor,
            tree.pane_left,
            tree.pane_right,
        );
    });
    let _ = reconcile_children(
        context,
        tree.pane_tree.stable_id(),
        &pane_tree_children(
            state,
            tree.pane_empty,
            tree.pane_editor,
            tree.pane_left,
            tree.pane_right,
        ),
    );
    let _ = context.update_component(tree.pane, |chrome, _| {
        chrome.actions = pane_actions(
            state,
            tree.pane_split.stable_id(),
            tree.pane_close.stable_id(),
        );
    });
    let _ = context.assemble_pane_chrome(tree.pane);
    let _ = context.update_component(tree.labeled, |value, _| {
        *value = selected_card_value(state);
    });
}

fn sync_feedback(
    context: &mut nana_ui::runtime::AppContext,
    tree: &FeedbackTree,
    state: &GalleryState,
) {
    let _ = context.update_component(tree.start, |button, _| {
        *button = start_loading_button(state);
    });
    let _ = context.update_component(tree.progress, |progress, _| {
        progress.value = progress_value(state);
    });
    let _ = context.update_component(tree.progress_label, |label, _| {
        *label = progress_label_text(state);
    });
    let _ = context.update_component(tree.spinner, |spinner, _| {
        *spinner = progress_spinner(state);
    });
    let _ = context.update_component(tree.skeleton, |skeleton, _| {
        *skeleton = progress_skeleton(state);
    });
    let _ = context.update_component(tree.badge, |badge, _| {
        *badge = StatusBadge::new(action_status(state), StatusTone::Info);
    });
    let _ = context.update_component(tree.toast, |toast, _| {
        *toast = Toast::new(action_status(state), ToastTone::Info);
    });
    let _ = context.update_component(tree.dialog, |button, _| {
        *button = dialog_button(state);
    });
    let _ = context.update_component(tree.popover, |popover, _| {
        popover.open = state.popover_open;
    });
    let _ = context.update_component(tree.popover_action, |button, _| {
        *button = popover_action_button(state.popover_open);
    });
}

fn sync_data(context: &mut nana_ui::runtime::AppContext, tree: &DataTree, state: &GalleryState) {
    let _ = context.update_component(tree.trend_status, |label, _| {
        *label = chart_status_text(state, GalleryChart::Trend);
    });
    let _ = context.update_component(tree.share_status, |label, _| {
        *label = chart_status_text(state, GalleryChart::Share);
    });
    let _ = context.update_component(tree.calendar_status, |label, _| {
        *label = calendar_status_text(state);
    });
}

fn sync_workspace(
    context: &mut nana_ui::runtime::AppContext,
    tree: &WorkspaceTree,
    state: &GalleryState,
) {
    let mut contents = std::collections::HashMap::new();
    for (id, panel) in &tree.panels {
        contents.insert(id.clone(), panel.stable_id());
    }
    let _ = context.update_component(tree.dock, |dock, _| {
        *dock = runtime_dock_from_workspace(state, &contents);
    });
    let locked = state.dock_locked;
    let hidden_assets = !state.dock_is_visible("gallery.assets");
    let _ = context.update_component(tree.lock, |button, _| {
        *button = Button::new(if locked { "解锁 Dock" } else { "锁定 Dock" })
            .kind(ButtonKind::Subtle)
            .size(ControlSize::Small);
    });
    let _ = context.update_component(tree.hide, |button, _| {
        *button = Button::new(if hidden_assets {
            "恢复 Assets"
        } else {
            "隐藏 Assets"
        })
        .kind(ButtonKind::Subtle)
        .size(ControlSize::Small);
    });
    let _ = context.update_component(tree.status, |label, _| {
        *label = workspace_status_text(dock_status(state));
    });
}

fn sync_inspector(
    context: &mut nana_ui::runtime::AppContext,
    tree: &InspectorTree,
    state: &GalleryState,
) {
    let radius = state.appearance.standard_radius().round() as u8;
    let _ = context.update_component(tree.radius, |range, _| {
        range.value = f64::from(radius);
    });
    let _ = context.update_component(tree.corners, |switch, _| {
        *switch = Switch::new("主区域圆角", state.appearance.workspace_corners_enabled());
    });
}

/// An equal-width cell of a row around `child`.
fn flex_cell<V: IntoView>(child: V) -> impl IntoView + use<V> {
    widget(HostStack::flex_child()).children(child)
}

fn apply_equal_fill(layout: &mut nana_ui::runtime::LayoutStyle, height: f32) {
    layout.width = Some(LengthSpec::Fill);
    layout.height = Some(LengthSpec::Px(height));
    layout.min_width = Some(LengthSpec::Px(0.0));
    layout.flex_grow = Some(1.0);
    layout.flex_shrink = Some(1.0);
    layout.allow_shrink = true;
}

fn fill_range_field(mut field: nana_ui::runtime::RangeField) -> nana_ui::runtime::RangeField {
    let layout = std::sync::Arc::make_mut(&mut field.style.layout);
    layout.width = Some(LengthSpec::Fill);
    layout.min_width = Some(LengthSpec::Px(0.0));
    layout.flex_grow = Some(1.0);
    layout.flex_shrink = Some(1.0);
    layout.allow_shrink = true;
    field
}

fn workspace_status_text(value: impl Into<String>) -> nana_ui::runtime::Text {
    let mut text = hugging_text(
        value,
        SemanticColorRole::Muted,
        type_scale::META,
        type_scale::REGULAR,
    );
    let layout = std::sync::Arc::make_mut(&mut text.style.layout);
    layout.white_space = nana_ui::runtime::WhiteSpaceSpec::Nowrap;
    layout.flex_grow = Some(1.0);
    layout.flex_shrink = Some(1.0);
    layout.min_width = Some(LengthSpec::Px(0.0));
    layout.allow_shrink = true;
    layout.text_overflow_ellipsis = true;
    text
}

fn popover_action_button(open: bool) -> Button {
    let mut button = Button::new("执行主要操作").kind(ButtonKind::Primary);
    let layout = std::sync::Arc::make_mut(&mut button.style.layout);
    layout.hidden = !open;
    button
}

fn list_slot_text(
    value: impl Into<String>,
    color: SemanticColorRole,
    size: f32,
    weight: u16,
    fill: bool,
) -> nana_ui::runtime::Text {
    let mut text = labeled_text(
        value,
        color,
        size,
        weight,
        Some(if fill {
            LengthSpec::Fill
        } else {
            LengthSpec::Shrink
        }),
    );
    let layout = std::sync::Arc::make_mut(&mut text.style.layout);
    layout.flex_grow = Some(if fill { 1.0 } else { 0.0 });
    layout.flex_shrink = Some(if fill { 1.0 } else { 0.0 });
    if fill {
        layout.min_width = Some(LengthSpec::Px(0.0));
        layout.allow_shrink = true;
    }
    layout.white_space = nana_ui::runtime::WhiteSpaceSpec::Nowrap;
    layout.text_overflow_ellipsis = fill;
    text
}

fn list_leading_text(selected: bool) -> nana_ui::runtime::Text {
    list_slot_text(
        if selected { "●" } else { "○" },
        if selected {
            SemanticColorRole::Accent
        } else {
            SemanticColorRole::Faint
        },
        10.0,
        400,
        false,
    )
}

fn list_label_text(label: &str) -> nana_ui::runtime::Text {
    list_slot_text(label, SemanticColorRole::Text, 13.0, 500, true)
}

fn list_trailing_text(meta: &str) -> nana_ui::runtime::Text {
    list_slot_text(meta, SemanticColorRole::Muted, type_scale::HINT, 400, false)
}

fn list_item_slots(
    leading: Entity<nana_ui::runtime::Text>,
    content: Entity<nana_ui::runtime::Text>,
    trailing: Entity<nana_ui::runtime::Text>,
) -> ListItemSlots {
    ListItemSlots {
        leading: Some(leading.stable_id()),
        content: Some(content.stable_id()),
        trailing: Some(trailing.stable_id()),
    }
}

fn gallery_list_item(
    label: &str,
    selected: bool,
    disabled: bool,
    leading: Entity<nana_ui::runtime::Text>,
    content: Entity<nana_ui::runtime::Text>,
    trailing: Entity<nana_ui::runtime::Text>,
) -> ListItem {
    list_item_spec(label, selected, disabled).slots(list_item_slots(leading, content, trailing))
}

/// A list row without its slots, which a view gives it as children.
fn list_item_spec(label: &str, selected: bool, disabled: bool) -> ListItem {
    let mut item = ListItem::new(label)
        .size(ControlSize::Small)
        .selected(selected)
        .disabled(disabled);
    let layout = std::sync::Arc::make_mut(&mut item.style.layout);
    layout.width = Some(LengthSpec::Fill);
    layout.min_width = Some(LengthSpec::Px(0.0));
    layout.flex_grow = Some(0.0);
    layout.flex_shrink = Some(0.0);
    layout.white_space = nana_ui::runtime::WhiteSpaceSpec::Nowrap;
    layout.text_overflow_ellipsis = true;
    item
}

/// A section page: a vertical scroll filling the primary region, holding the
/// page's header and its rows of cards.
fn page_scroll() -> ScrollView {
    let mut style = NodeStyle {
        background: Some(SemanticColorRole::Background),
        ..NodeStyle::default()
    };
    let layout = Arc::make_mut(&mut style.layout);
    layout.width = Some(LengthSpec::Fill);
    layout.height = Some(LengthSpec::Fill);
    layout.min_width = Some(LengthSpec::Px(0.0));
    layout.min_height = Some(LengthSpec::Px(0.0));
    layout.flex_grow = Some(1.0);
    layout.flex_shrink = Some(1.0);
    layout.allow_shrink = true;
    layout.border_radius = Some(8.0);
    ScrollView::new(ScrollAxes::Vertical).style(style)
}

fn page_body() -> HostStack {
    HostStack::column(16.0).padding_xy(20.0, 18.0)
}

/// The page's title and the one line under it.
fn page_header(title: &'static str, description: &'static str) -> impl IntoView {
    widget(HostStack::column(4.0).min_width(LengthSpec::Px(0.0))).children((
        widget(styled_text(
            title,
            SemanticColorRole::Text,
            type_scale::TITLE,
            type_scale::SEMIBOLD,
        )),
        widget(styled_text(
            description,
            SemanticColorRole::Muted,
            type_scale::META,
            type_scale::REGULAR,
        )),
    ))
}

/// Cards side by side, sharing the row's width and its tallest height. A
/// card spanning the page goes in a row of its own too: a card grows along
/// its parent's axis, and down the page's auto-height column the layout
/// engine shares that column's height out evenly among growing children
/// instead of sizing each to its content.
fn card_row<V: IntoView>(cards: V) -> impl IntoView + use<V> {
    widget(HostStack::fill_row(16.0)).children(cards)
}

/// One feature group: a titled card, a line saying what it shows, then the
/// controls. It grows along whichever axis its parent lays out.
fn showcase<B: IntoView>(
    title: &'static str,
    description: &'static str,
    body: B,
) -> impl IntoView + use<B> {
    let mut card = Card::new().title(title);
    let layout = Arc::make_mut(&mut card.style.layout);
    layout.direction = Some(nana_ui::runtime::FlexDirection::Column);
    layout.gap = Some(LengthSpec::Px(12.0));
    layout.align_items = nana_ui::runtime::AlignSpec::Stretch;
    layout.width = Some(LengthSpec::Fill);
    layout.min_width = Some(LengthSpec::Px(0.0));
    layout.flex_grow = Some(1.0);
    layout.flex_shrink = Some(1.0);
    layout.allow_shrink = true;
    widget(card).children((
        widget(styled_text(
            description,
            SemanticColorRole::Muted,
            type_scale::HINT,
            type_scale::REGULAR,
        )),
        body,
    ))
}

fn click_count_text(state: &GalleryState) -> nana_ui::runtime::Text {
    styled_text(
        format!("主要操作已触发 {} 次", state.primary_clicks),
        SemanticColorRole::Muted,
        type_scale::HINT,
        type_scale::REGULAR,
    )
}

fn selection_status_text(state: &GalleryState) -> nana_ui::runtime::Text {
    let mut chosen = state.dropdown_values.clone();
    chosen.sort_unstable();
    let chosen = chosen
        .iter()
        .map(|value| dropdown_label(*value))
        .collect::<Vec<_>>();
    let dropdown = if chosen.is_empty() {
        "下拉未选择".to_owned()
    } else {
        format!("下拉已选：{}", chosen.join("、"))
    };
    let search = state
        .search_selection
        .and_then(|value| {
            state
                .search_dropdown_options
                .iter()
                .find(|option| option.value.as_ref() == value.to_string())
        })
        .map_or("搜索未选择".to_owned(), |option| {
            format!("搜索已选：{}", option.label)
        });
    styled_text(
        format!("{dropdown} · {search}"),
        SemanticColorRole::Muted,
        type_scale::HINT,
        type_scale::REGULAR,
    )
}

fn dropdown_label(value: u8) -> &'static str {
    match value {
        0 => "关闭",
        50 => "平衡",
        _ => "最大",
    }
}

fn xy_label_text(state: &GalleryState) -> nana_ui::runtime::Text {
    styled_text(
        format!("X {:.2} · Y {:.2}", state.xy_pad.x, state.xy_pad.y),
        SemanticColorRole::Muted,
        type_scale::HINT,
        type_scale::REGULAR,
    )
}

fn selected_card_value(state: &GalleryState) -> LabeledValue {
    let name = SURFACE_CARDS
        .get(state.selected_surface_card)
        .map_or("无", |(title, _)| *title);
    LabeledValue::new("当前卡片", name)
}

fn start_loading_button(state: &GalleryState) -> Button {
    Button::new(if state.loading {
        "处理中"
    } else {
        "开始加载"
    })
    .kind(ButtonKind::Primary)
    .loading(state.loading)
}

/// How far the loading cycle is: `LoadingTick`s out of the twelve it takes.
fn progress_value(state: &GalleryState) -> f64 {
    if state.loading {
        f64::from(state.loading_ticks) / 12.0 * 100.0
    } else {
        0.0
    }
}

fn progress_label_text(state: &GalleryState) -> nana_ui::runtime::Text {
    styled_text(
        if state.loading {
            format!("已完成 {:.0}%", progress_value(state))
        } else {
            "空闲".to_owned()
        },
        SemanticColorRole::Muted,
        type_scale::HINT,
        type_scale::REGULAR,
    )
}

/// Spins only while the cycle runs.
fn progress_spinner(state: &GalleryState) -> Spinner {
    let mut spinner = Spinner::new("处理中");
    Arc::make_mut(&mut spinner.style.layout).hidden = !state.loading;
    spinner
}

/// Holds the place of the content the cycle is fetching, while it runs.
fn progress_skeleton(state: &GalleryState) -> Skeleton {
    let mut skeleton = Skeleton::fill_width(10.0);
    Arc::make_mut(&mut skeleton.style.layout).hidden = !state.loading;
    skeleton
}

fn drop_zone() -> Card {
    let mut zone = Card::new().kind(CardKind::Outlined);
    let layout = Arc::make_mut(&mut zone.style.layout);
    layout.min_height = Some(LengthSpec::Px(72.0));
    layout.justify_content = nana_ui::runtime::JustifySpec::Center;
    zone
}

fn dialog_button(state: &GalleryState) -> Button {
    Button::new(if state.overlay.contains(&super::GalleryOverlay::Dialog) {
        "关闭对话框"
    } else {
        "打开对话框"
    })
    .kind(ButtonKind::Primary)
}

fn link_status_text(state: &GalleryState) -> nana_ui::runtime::Text {
    styled_text(
        state
            .opened_markdown_link
            .as_ref()
            .map_or(String::new(), |link| format!("已选择链接：{link}")),
        SemanticColorRole::Accent,
        type_scale::HINT,
        type_scale::REGULAR,
    )
}

const TREND_DAYS: [&str; 7] = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];
const TREND_SERIES: [(&str, [f64; 7]); 2] = [
    ("访问", [120.0, 200.0, 150.0, 80.0, 70.0, 110.0, 130.0]),
    ("转化", [80.0, 132.0, 101.0, 64.0, 50.0, 90.0, 96.0]),
];
const SHARE_ITEMS: [(&str, f64); 4] = [
    ("文本", 42.0),
    ("图像", 26.0),
    ("图表", 18.0),
    ("其他", 14.0),
];

fn chart_style() -> NodeStyle {
    let mut style = NodeStyle::default();
    Arc::make_mut(&mut style.layout).height = Some(LengthSpec::Px(240.0));
    style
}

fn trend_chart_option() -> nana_ui::runtime::chart::ChartOption {
    use nana_ui::runtime::chart::{Axis, BarSeries, ChartOption, Legend, LineSeries};
    let [(visits, visit_data), (conversions, conversion_data)] = TREND_SERIES;
    ChartOption::new()
        .legend(Legend::default())
        .x_axis(Axis::category(TREND_DAYS))
        .y_axis(Axis::value())
        .series(BarSeries::new(visits, visit_data))
        .series(LineSeries::new(conversions, conversion_data).smooth(true))
}

fn share_chart_option() -> nana_ui::runtime::chart::ChartOption {
    use nana_ui::runtime::chart::{ChartOption, Legend, PieItem, PieSeries};
    ChartOption::new().legend(Legend::default()).series(
        PieSeries::new(
            "构成",
            SHARE_ITEMS
                .iter()
                .map(|(name, value)| PieItem::new(*name, *value)),
        )
        .ring(0.5, 0.78),
    )
}

fn chart_message(chart: GalleryChart, event: &ChartEvent) -> GalleryMessage {
    match event {
        ChartEvent::Click { series, index } => GalleryMessage::SelectChartItem {
            chart,
            series: *series,
            index: *index,
        },
        _ => GalleryMessage::OverlayInteraction,
    }
}

fn chart_status_text(state: &GalleryState, chart: GalleryChart) -> nana_ui::runtime::Text {
    let selected = state
        .chart_selection
        .filter(|(selected, _, _)| *selected == chart)
        .and_then(|(_, series, index)| match chart {
            GalleryChart::Trend => {
                let (name, data) = TREND_SERIES.get(series)?;
                let day = TREND_DAYS.get(index)?;
                Some(format!("{day} · {name} {}", data.get(index)?))
            }
            GalleryChart::Share => {
                let total: f64 = SHARE_ITEMS.iter().map(|(_, value)| value).sum();
                let (name, value) = SHARE_ITEMS.get(index)?;
                Some(format!("{name} · 占比 {:.0}%", value / total * 100.0))
            }
        });
    match selected {
        Some(copy) => styled_text(
            copy,
            SemanticColorRole::Accent,
            type_scale::HINT,
            type_scale::MEDIUM,
        ),
        None => styled_text(
            "未选择",
            SemanticColorRole::Muted,
            type_scale::HINT,
            type_scale::REGULAR,
        ),
    }
}

fn calendar_status_text(state: &GalleryState) -> nana_ui::runtime::Text {
    styled_text(
        state
            .calendar_active
            .as_ref()
            .map_or("移动指针查看日期".to_owned(), |cell| {
                cell.title.clone()
            }),
        SemanticColorRole::Muted,
        type_scale::HINT,
        type_scale::REGULAR,
    )
}

fn loading_button(state: &GalleryState) -> Button {
    Button::new(if state.loading { "处理中" } else { "加载" })
        .kind(ButtonKind::Text)
        .loading(state.loading)
}

fn gallery_dropdown(state: &GalleryState, placeholder: &str, size: ControlSize) -> Dropdown {
    Dropdown::multiple(state.dropdown_values.iter().map(|value| value.to_string()))
        .options([
            DropdownOption::new("0", "关闭"),
            DropdownOption::new("50", "平衡"),
            DropdownOption::new("100", "最大"),
        ])
        .placeholder(placeholder.to_owned())
        .size(size)
}

fn gallery_search(state: &GalleryState) -> SearchDropdown {
    SearchDropdown::new(state.search_selection.map(|value| value.to_string()))
        .options(state.search_dropdown_options.iter().map(|option| {
            let mut item =
                SearchDropdownOption::new(option.value.to_string(), option.label.clone());
            if let Some(hint) = &option.hint {
                item = item.hint(hint.clone());
            }
            item
        }))
        .placeholder("搜索选项")
        .query(state.search_dropdown_query.clone())
}

fn gallery_textarea(state: &GalleryState) -> TextArea {
    TextArea::new(state.editor.as_str())
        .placeholder("输入说明")
        .height(96.0)
        .invalid(state.editor.trim().chars().count() < 4)
        .disabled(!state.editor_enabled())
}

fn gallery_code_editor(state: &GalleryState) -> TextArea {
    TextArea::new(state.editor.as_str())
        .placeholder("fn main() {}")
        .height(96.0)
        .line_numbers(true)
        .minimap(true)
        .diagnostics(std::sync::Arc::from([
            nana_ui::runtime::TextDiagnosticSpan::new(
                0,
                2,
                nana_ui::runtime::TextDiagnosticSeverity::Error,
            )
            .with_message("示例诊断"),
        ]))
        .git_gutter(std::sync::Arc::from([nana_ui::runtime::TextGitMark::new(
            1,
            nana_ui::runtime::TextGitMarkKind::Modified,
        )]))
        .disabled(!state.editor_enabled())
}

fn gallery_terminal_view() -> TerminalView {
    let mut view = TerminalView::new(gallery_terminal_screen());
    let layout = Arc::make_mut(&mut view.style.layout);
    layout.height = Some(LengthSpec::Px(
        view.cell_height * f32::from(view.screen.rows),
    ));
    layout.min_height = layout.height;
    layout.flex_grow = Some(0.0);
    layout.flex_shrink = Some(0.0);
    view
}

fn gallery_diff_view() -> DiffView {
    let mut view = DiffView::new(gallery_diff_hunks());
    let layout = Arc::make_mut(&mut view.style.layout);
    layout.height = Some(LengthSpec::Shrink);
    layout.min_height = Some(LengthSpec::Px(80.0));
    layout.flex_grow = Some(0.0);
    layout.flex_shrink = Some(0.0);
    view
}

fn field_status_text(state: &GalleryState) -> nana_ui::runtime::Text {
    let invalid = state.input.trim().is_empty();
    styled_text(
        if invalid {
            "请输入名称"
        } else {
            "名称可用"
        },
        if invalid {
            SemanticColorRole::Danger
        } else {
            SemanticColorRole::Success
        },
        12.0,
        400,
    )
}

fn editor_status_text(state: &GalleryState) -> nana_ui::runtime::Text {
    let invalid = state.editor.trim().chars().count() < 4;
    let (copy, color) = if invalid {
        ("请至少输入 4 个字符", SemanticColorRole::Danger)
    } else if state.editor_enabled() {
        ("说明可编辑", SemanticColorRole::Muted)
    } else if !state.checked {
        ("选项停用时不可编辑", SemanticColorRole::Muted)
    } else {
        ("说明已锁定", SemanticColorRole::Muted)
    };
    styled_text(copy, color, type_scale::META, type_scale::REGULAR)
}

fn gallery_tree(state: &GalleryState) -> TreeView {
    TreeView::new([
        TreeNode::branch(
            Arc::<str>::from("src"),
            "src",
            state.tree_expanded,
            [
                TreeNode::leaf(Arc::<str>::from("src/lib.rs"), "lib.rs").icon(Icon::File),
                TreeNode::leaf(Arc::<str>::from("src/main.rs"), "main.rs").icon(Icon::File),
            ],
        )
        .icon(Icon::Folder),
        TreeNode::leaf(Arc::<str>::from("README.md"), "README.md").icon(Icon::File),
    ])
    .selection({
        let mut selection = SelectionState::single();
        selection.select_only(Arc::<str>::from(state.tree_selected.as_str()));
        selection
    })
}

fn pane_tree_node(
    state: &GalleryState,
    empty: Entity<nana_ui::runtime::Text>,
    editor: Entity<nana_ui::runtime::Text>,
    left: Entity<nana_ui::runtime::Text>,
    right: Entity<nana_ui::runtime::Text>,
) -> PaneTreeNode {
    pane_tree_node_for(
        state.pane_chrome_item_open,
        state.pane_chrome_split,
        empty,
        editor,
        left,
        right,
    )
}

fn pane_tree_node_for(
    item_open: bool,
    split: bool,
    empty: Entity<nana_ui::runtime::Text>,
    editor: Entity<nana_ui::runtime::Text>,
    left: Entity<nana_ui::runtime::Text>,
    right: Entity<nana_ui::runtime::Text>,
) -> PaneTreeNode {
    if !item_open {
        PaneTreeNode::leaf_content("empty", empty.stable_id())
    } else if split {
        PaneTreeNode::split(
            "editor-split",
            nana_ui::SplitAxis::Horizontal,
            0.5,
            PaneTreeNode::leaf_content("left", left.stable_id()),
            PaneTreeNode::leaf_content("right", right.stable_id()),
        )
    } else {
        PaneTreeNode::leaf_content("editor", editor.stable_id())
    }
}

fn pane_tree_children(
    state: &GalleryState,
    empty: Entity<nana_ui::runtime::Text>,
    editor: Entity<nana_ui::runtime::Text>,
    left: Entity<nana_ui::runtime::Text>,
    right: Entity<nana_ui::runtime::Text>,
) -> Vec<StableNodeId> {
    if !state.pane_chrome_item_open {
        vec![empty.stable_id()]
    } else if state.pane_chrome_split {
        vec![left.stable_id(), right.stable_id()]
    } else {
        vec![editor.stable_id()]
    }
}

fn pane_actions(
    state: &GalleryState,
    split: StableNodeId,
    close: StableNodeId,
) -> Vec<PaneChromeAction> {
    pane_actions_for(
        state.pane_chrome_item_open,
        state.pane_chrome_split,
        split,
        close,
    )
}

fn pane_actions_for(
    item_open: bool,
    split_open: bool,
    split: StableNodeId,
    close: StableNodeId,
) -> Vec<PaneChromeAction> {
    let mut actions = Vec::new();
    if item_open && !split_open {
        actions.push(
            PaneChromeAction::new(PaneChromeActionKind::SplitHorizontal, "左右分栏").target(split),
        );
    }
    if item_open {
        actions.push(
            PaneChromeAction::new(PaneChromeActionKind::CloseItem, "关闭 Item")
                .icon(Icon::Close)
                .target(close),
        );
    }
    actions
}

fn runtime_dock_from_workspace(
    state: &GalleryState,
    contents: &std::collections::HashMap<String, StableNodeId>,
) -> nana_ui::runtime::Dock {
    runtime_dock_from_node(state, &state.dock.main, contents)
}

fn runtime_dock_from_node(
    state: &GalleryState,
    root: &nana_ui::runtime::DockNode,
    contents: &std::collections::HashMap<String, StableNodeId>,
) -> nana_ui::runtime::Dock {
    let root = bind_dock_contents(root, contents);
    let mut view = nana_ui::runtime::Dock::new(root).locked(state.dock_locked);
    if let Some(primary) = state.dock.primary.as_deref() {
        view = view.primary(primary);
    }
    view.hidden.clone_from(&state.dock.hidden);
    for (id, title) in DOCK_TITLES {
        view = view.title(id, title);
    }
    view
}

fn dock_tree_without_contents(node: &nana_ui::runtime::DockNode) -> nana_ui::runtime::DockNode {
    match node {
        nana_ui::runtime::DockNode::Item { id, .. } => {
            nana_ui::runtime::DockNode::item(Arc::clone(id), None)
        }
        nana_ui::runtime::DockNode::Tabs { tabs, active, .. } => nana_ui::runtime::DockNode::tabs(
            tabs.iter().cloned(),
            Arc::clone(active),
            tabs.iter().map(|id| (Arc::clone(id), None)),
        ),
        nana_ui::runtime::DockNode::Split {
            axis,
            ratio,
            first,
            second,
        } => nana_ui::runtime::DockNode::split(
            *axis,
            *ratio,
            dock_tree_without_contents(first),
            dock_tree_without_contents(second),
        ),
    }
}

fn floated_runtime_dock_ids(
    previous: &nana_ui::runtime::DockNode,
    next: &nana_ui::runtime::DockNode,
    hidden: &[Arc<str>],
) -> Vec<Arc<str>> {
    previous
        .flatten()
        .into_iter()
        .filter(|id| {
            !next.contains(id.as_ref())
                && hidden.iter().all(|hidden| hidden.as_ref() != id.as_ref())
        })
        .collect()
}

fn bind_dock_contents(
    node: &nana_ui::runtime::DockNode,
    contents: &std::collections::HashMap<String, StableNodeId>,
) -> nana_ui::runtime::DockNode {
    match node {
        nana_ui::runtime::DockNode::Item { id, content } => nana_ui::runtime::DockNode::item(
            Arc::clone(id),
            contents.get(id.as_ref()).copied().or(*content),
        ),
        nana_ui::runtime::DockNode::Tabs {
            tabs,
            active,
            contents: tab_contents,
        } => {
            let pairs = tabs
                .iter()
                .map(|id| {
                    let content = contents.get(id.as_ref()).copied().or_else(|| {
                        tab_contents
                            .iter()
                            .find(|(tab, _)| tab == id)
                            .and_then(|(_, content)| *content)
                    });
                    (Arc::clone(id), content)
                })
                .collect::<Vec<_>>();
            nana_ui::runtime::DockNode::tabs(tabs.iter().cloned(), Arc::clone(active), pairs)
        }
        nana_ui::runtime::DockNode::Split {
            axis,
            ratio,
            first,
            second,
        } => nana_ui::runtime::DockNode::split(
            *axis,
            *ratio,
            bind_dock_contents(first, contents),
            bind_dock_contents(second, contents),
        ),
    }
}

fn selected_dock_tab(
    context: &nana_ui::runtime::AppContext,
    dock: &nana_ui::runtime::Dock,
) -> Option<String> {
    let _ = (context, dock);
    None
}

fn map_dropdown_event(event: &DropdownEvent<Arc<str>>) -> GalleryMessage {
    let parse = |value: &str| value.parse::<u8>().unwrap_or(0);
    match event {
        DropdownEvent::Select(value) => {
            GalleryMessage::SetDropdown(nana_ui::runtime::DropdownEvent::Select(parse(value)))
        }
        DropdownEvent::Toggle(value) => {
            GalleryMessage::SetDropdown(nana_ui::runtime::DropdownEvent::Toggle(parse(value)))
        }
        DropdownEvent::Opened => {
            GalleryMessage::SetDropdown(nana_ui::runtime::DropdownEvent::Opened)
        }
        DropdownEvent::Closed => {
            GalleryMessage::SetDropdown(nana_ui::runtime::DropdownEvent::Closed)
        }
    }
}

fn map_search_event(event: &SearchDropdownEvent) -> GalleryMessage {
    match event {
        SearchDropdownEvent::Search(query) => GalleryMessage::SearchDropdownInput(query.clone()),
        SearchDropdownEvent::Select(value) => {
            GalleryMessage::SelectSearchResult(value.parse().unwrap_or(0))
        }
        SearchDropdownEvent::Opened | SearchDropdownEvent::Closed => {
            GalleryMessage::OverlayInteraction
        }
    }
}

fn gallery_calendar_data() -> Vec<CalendarHeatmapDatum> {
    (0..84)
        .map(|offset| {
            let day = 1 + offset;
            let month = 4 + (day - 1) / 30;
            let day_of_month = 1 + (day - 1) % 30;
            CalendarHeatmapDatum::new(
                format!("2026-{month:02}-{day_of_month:02}"),
                ((offset * 7 + 3) % 18) as f32,
            )
        })
        .collect()
}

fn graph_selection_label(state: &GalleryState) -> String {
    match state.graph_selection.as_ref() {
        Some(nana_ui::GraphSelection::Node(node)) => format!("节点 · {node}"),
        Some(nana_ui::GraphSelection::Port { node, port }) => format!("端口 · {node} / {port}"),
        Some(nana_ui::GraphSelection::Edge(edge)) => format!("连线 · {edge}"),
        None => "未选择".to_owned(),
    }
}

fn action_status(state: &GalleryState) -> String {
    match state.context_action {
        Some(super::ContextAction::Duplicate) => "已复制".to_owned(),
        Some(super::ContextAction::Rename) => "已重命名".to_owned(),
        Some(super::ContextAction::Remove) => "已移除".to_owned(),
        None if state.confirmed_actions > 0 => {
            format!("操作已确认 {} 次", state.confirmed_actions)
        }
        None => "等待操作".to_owned(),
    }
}

fn dock_status(state: &GalleryState) -> String {
    format!(
        "拖动分隔条调整，双击复位；当前浮窗 {} 个。",
        state.dock.floating.len()
    )
}
