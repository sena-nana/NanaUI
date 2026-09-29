use std::fmt;
use std::sync::{Arc, Mutex};

use nana_ui::runtime::view::{EntityRef, IntoView, detached, entity_ref, widget, with_refs};
use nana_ui::runtime::{
    Activate, AppShell, AppTitleBar, Avatar, Button, CalendarHeatmap, CalendarHeatmapDatum,
    CalendarHeatmapEvent, Card, Checkbox, Chip, DesktopShell, DiffHunk, DiffLine, DiffView,
    DockFloatingSurface, DocumentId, DropAccepts, Dropdown, DropdownEvent, DropdownOption,
    EmptyState, Entity, FileDropEvent, FrameworkError, GraphCanvas, GraphCanvasEvent, GraphMinimap,
    GraphMinimapEvent, GraphSize, IconButton, InteractiveCard, LabeledValue, LayoutViewport,
    LengthSpec, LevelMeter, ListItem, ListItemSlots, NativeMarkdown, NodeStyle, OverlayHost,
    PaneChrome, PaneChromeAction, PaneChromeActionKind, PaneTree, PaneTreeNode, Popover,
    PopoverClosed, PopoverToggled, PositionSpec, Progress, RangeInput, RichTextEvent,
    RuntimeDocument, SearchDropdown, SearchDropdownEvent, SearchDropdownOption, SegmentedControl,
    SegmentedOption, SegmentedSelectionRequested, SemanticColorRole, SidebarFooter,
    SidebarFooterButton, SidebarFrame, SidebarRow, SidebarRowIcon, SidebarRowState, SidebarSection,
    Skeleton, Spinner, StableNodeId, StatusBadge, Switch, TabOption, Tabs, TabsEvent,
    TerminalScreen, TerminalView, Text, TextArea, TextChanged, TextInput, Thumbnail, Toast,
    ToggleChanged, TreeNode, TreeView, TreeViewEvent, ValidationMessage, XYPad, XYPadEvent,
};
use nana_ui::theme::type_scale;
use nana_ui::{
    ButtonKind, CardKind, ControlSize, Icon, NanaTextShaper, RegionId, StatusTone, ToastTone,
    ValidationIntent, WorkspaceAction, WorkspaceModel,
};
use nana_ui_platform::InputPayload;

use super::runtime_host::{
    DEFAULT_VIEWPORT, HostStack, RuntimeChrome, RuntimeSceneInput, ScriptedInput,
    apply_title_bar_maximized, apply_workspace_corners, bind_event, event_point, hugging_text,
    labeled_text, node_is_or_under, queue, reconcile_children, runtime_input_event,
    search_command_button, sidebar_toggle_button, styled_text, take_pending, theme_toggle_button,
};
use super::{
    GalleryDock, GalleryMessage, GallerySection, GalleryState, SurfaceView, section_label,
};

type SidebarMount = (
    Entity<SidebarFrame>,
    [Entity<SidebarRow>; 6],
    Entity<SidebarFooterButton>,
);
type RichTextMount = (
    Entity<HostStack>,
    Entity<NativeMarkdown>,
    Entity<nana_ui::runtime::Text>,
    Entity<HostStack>,
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

const SECTIONS: [(GallerySection, &str, Icon); 6] = [
    (GallerySection::Controls, "控件", Icon::Settings),
    (GallerySection::Surfaces, "表面", Icon::Folder),
    (GallerySection::Feedback, "反馈", Icon::About),
    (GallerySection::RichText, "富文本", Icon::About),
    (GallerySection::Graph, "节点图", Icon::Nodes),
    (GallerySection::Workspace, "工作区", Icon::Workspace),
];

const LIST_ITEMS: [(&str, bool, ControlSize); 16] = [
    ("小档列表项", false, ControlSize::Small),
    ("中档列表项", false, ControlSize::Medium),
    ("大档列表项", false, ControlSize::Large),
    ("带辅助信息", false, ControlSize::Small),
    ("紧凑列表项", false, ControlSize::Small),
    ("长文本列表项", false, ControlSize::Small),
    ("可操作列表项", false, ControlSize::Small),
    ("禁用列表项", true, ControlSize::Small),
    ("普通状态", false, ControlSize::Small),
    ("悬停状态", false, ControlSize::Small),
    ("按下状态", false, ControlSize::Small),
    ("成功状态", false, ControlSize::Small),
    ("警告状态", false, ControlSize::Small),
    ("错误状态", false, ControlSize::Small),
    ("加载状态", false, ControlSize::Small),
    ("空状态", false, ControlSize::Small),
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

/// Gallery snapshot is 1280×800. DesktopShell chrome:
/// title 36 + PrimaryToolbar 34 + Diagnostics 180 → primary 550.
/// Canvas padding/gap + hugged tools/popup consume 230; 430 dock cannot fit.
const WORKSPACE_CANVAS_PADDING: f32 = 8.0;
const WORKSPACE_CANVAS_GAP: f32 = 8.0;
const WORKSPACE_DOCK_HEIGHT: f32 = 320.0;
const WORKSPACE_POPUP_WIDTH: f32 = 360.0;
const WORKSPACE_POPUP_HEIGHT: f32 = 150.0;
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
    sidebar_rows: [Entity<SidebarRow>; 6],
    settings_footer: Entity<SidebarFooterButton>,
    controls: ControlsTree,
    surfaces: SurfacesTree,
    feedback: FeedbackTree,
    rich_text_root: Entity<HostStack>,
    rich_text: Entity<NativeMarkdown>,
    link_status: Entity<nana_ui::runtime::Text>,
    #[cfg_attr(not(test), allow(dead_code))]
    drop: Entity<HostStack>,
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
    root: Entity<HostStack>,
    _small: Entity<Button>,
    _medium: Entity<Button>,
    _large: Entity<Button>,
    loading: Entity<Button>,
    _add: Entity<IconButton>,
    clicks: Entity<nana_ui::runtime::Text>,
    segmented: [Entity<SegmentedControl>; 3],
    segmented_on: [Entity<SegmentedOption>; 3],
    segmented_off: [Entity<SegmentedOption>; 3],
    inputs: [Entity<TextInput>; 3],
    secure: Entity<TextInput>,
    dropdowns: [Entity<Dropdown>; 3],
    field_status: Entity<nana_ui::runtime::Text>,
    checkbox: Entity<Checkbox>,
    switch: Entity<Switch>,
    range: Entity<nana_ui::runtime::RangeField>,
    search: Entity<SearchDropdown>,
    textarea: Entity<TextArea>,
    editor_status: Entity<nana_ui::runtime::Text>,
    xy_pad: Entity<XYPad>,
    xy_label: Entity<nana_ui::runtime::Text>,
    list_items: Vec<Entity<ListItem>>,
    list_leads: Vec<Entity<nana_ui::runtime::Text>>,
    list_labels: Vec<Entity<nana_ui::runtime::Text>>,
    list_trails: Vec<Entity<nana_ui::runtime::Text>>,
}

struct SurfacesTree {
    root: Entity<HostStack>,
    tabs: Entity<Tabs>,
    surface_row: Entity<HostStack>,
    overview: [Entity<Card>; 3],
    cards: [Entity<InteractiveCard>; 3],
    tree: Entity<TreeView>,
    pane: Entity<PaneChrome>,
    pane_tabs: Entity<nana_ui::runtime::Text>,
    pane_tree: Entity<PaneTree>,
    pane_empty: Entity<nana_ui::runtime::Text>,
    pane_editor: Entity<nana_ui::runtime::Text>,
    pane_left: Entity<nana_ui::runtime::Text>,
    pane_right: Entity<nana_ui::runtime::Text>,
    pane_split: Entity<Button>,
    pane_close: Entity<IconButton>,
    _empty: Entity<EmptyState>,
    labeled: Entity<LabeledValue>,
}

struct FeedbackTree {
    root: Entity<HostStack>,
    progress: Entity<Progress>,
    spinner: Entity<Spinner>,
    _skeleton: Entity<Skeleton>,
    meter: Entity<LevelMeter>,
    badge: Entity<StatusBadge>,
    _validation: Entity<ValidationMessage>,
    toast: Entity<Toast>,
    dialog: Entity<Button>,
    context: Entity<Button>,
    _image: Entity<Button>,
    popover: Entity<Popover>,
    popover_action: Entity<Button>,
    calendar: Entity<CalendarHeatmap>,
    calendar_status: Entity<nana_ui::runtime::Text>,
    action_status: Entity<nana_ui::runtime::Text>,
}

struct WorkspaceTree {
    root: Entity<HostStack>,
    dock: Entity<nana_ui::runtime::Dock>,
    panels: Vec<(String, Entity<HostStack>)>,
    lock: Entity<Button>,
    hide: Entity<Button>,
    _reset: Entity<Button>,
    status: Entity<nana_ui::runtime::Text>,
    _popup: Entity<AppShell>,
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
        let _ = context.set_theme(state.theme);

        let (sidebar, sidebar_rows, settings_footer) = mount_sidebar(context, document_id, state)?;
        let controls = mount_controls(context, document_id, state, &pending)?;
        let surfaces = mount_surfaces(context, document_id, state, &pending)?;
        let feedback = mount_feedback(context, document_id, state, &pending)?;
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
        let _ = context.set_theme(state.theme);
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
        let _ = context.update_component(self.rich_text, |markdown, _| {
            *markdown = state.markdown.clone();
        });
        let _ = context.assemble_markdown(self.rich_text);
        let _ = context.update_component(self.link_status, |label, _| {
            *label = match &state.opened_markdown_link {
                Some(link) => styled_text(
                    format!("已选择链接：{link}"),
                    SemanticColorRole::Accent,
                    type_scale::HINT,
                    type_scale::REGULAR,
                ),
                None => styled_text(
                    "",
                    SemanticColorRole::Muted,
                    type_scale::HINT,
                    type_scale::REGULAR,
                ),
            };
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
                let Some(bounds) = context
                    .world()
                    .layout_box(self.feedback.calendar.stable_id())
                else {
                    return Vec::new();
                };
                let hit = context
                    .read(self.feedback.calendar, |heatmap| {
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
                        target.unwrap_or(self.feedback.calendar.stable_id()),
                        self.feedback.calendar.stable_id(),
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
                        && let Some(bounds) = context.world().layout_box(self.rich_text.stable_id())
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
        let bounds = self
            .document
            .context()
            .world()
            .layout_box(self.drop.stable_id())?;
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
        let _ = context.set_theme(state.theme);
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
        let _ = context.set_theme(state.theme);
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

fn section_root(
    section: GallerySection,
    controls: &ControlsTree,
    surfaces: &SurfacesTree,
    feedback: &FeedbackTree,
    rich_text: Entity<HostStack>,
    graph: Entity<HostStack>,
    workspace: &WorkspaceTree,
) -> StableNodeId {
    match section {
        GallerySection::Controls => controls.root.stable_id(),
        GallerySection::Surfaces => surfaces.root.stable_id(),
        GallerySection::Feedback => feedback.root.stable_id(),
        GallerySection::RichText => rich_text.stable_id(),
        GallerySection::Graph => graph.stable_id(),
        GallerySection::Workspace => workspace.root.stable_id(),
    }
}

fn mount_sidebar(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    state: &GalleryState,
) -> Result<SidebarMount, FrameworkError> {
    let refs = std::cell::Cell::new(None);
    context.mount_view_detached(document_id, || {
        let frame = entity_ref::<SidebarFrame>();
        let rows: [EntityRef<SidebarRow>; 6] = std::array::from_fn(|_| entity_ref());
        let settings = entity_ref::<SidebarFooterButton>();
        refs.set(Some((frame, rows, settings)));
        let row_views = SECTIONS
            .iter()
            .zip(rows)
            .enumerate()
            .map(|(index, ((target, label, icon), row))| {
                let row_state = if state.section == *target {
                    SidebarRowState::Active
                } else {
                    SidebarRowState::Idle
                };
                widget(SidebarRow::new(*label).state(row_state))
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
        widget(SidebarFrame::new())
            .entity_ref(frame)
            .body(widget(SidebarSection::new("Gallery").count(6)).children(row_views))
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
        let root = entity_ref::<HostStack>();
        let buttons: [EntityRef<Button>; 4] = std::array::from_fn(|_| entity_ref());
        let add = entity_ref::<IconButton>();
        let clicks = entity_ref::<Text>();
        let segmented: [EntityRef<SegmentedControl>; 3] = std::array::from_fn(|_| entity_ref());
        let segmented_on: [EntityRef<SegmentedOption>; 3] = std::array::from_fn(|_| entity_ref());
        let segmented_off: [EntityRef<SegmentedOption>; 3] = std::array::from_fn(|_| entity_ref());
        let inputs: [EntityRef<TextInput>; 4] = std::array::from_fn(|_| entity_ref());
        let dropdowns: [EntityRef<Dropdown>; 3] = std::array::from_fn(|_| entity_ref());
        let texts: [EntityRef<Text>; 3] = std::array::from_fn(|_| entity_ref());
        let [field_status, editor_status, xy_label] = texts;
        let checkbox = entity_ref::<Checkbox>();
        let switch = entity_ref::<Switch>();
        let range = entity_ref::<nana_ui::runtime::RangeField>();
        let search = entity_ref::<SearchDropdown>();
        let textarea = entity_ref::<TextArea>();
        let xy_pad = entity_ref::<XYPad>();
        let list_items: Vec<EntityRef<ListItem>> =
            LIST_ITEMS.iter().map(|_| entity_ref()).collect();
        let list_leads: Vec<EntityRef<Text>> = LIST_ITEMS.iter().map(|_| entity_ref()).collect();
        let list_labels: Vec<EntityRef<Text>> = LIST_ITEMS.iter().map(|_| entity_ref()).collect();
        let list_trails: Vec<EntityRef<Text>> = LIST_ITEMS.iter().map(|_| entity_ref()).collect();
        let [small, medium, large, loading] = buttons;

        let primary = |label: &'static str, size, kind, button: EntityRef<Button>| {
            widget(Button::new(label).size(size).kind(kind))
                .entity_ref(button)
                .on(queue(pending, |_: &Activate| GalleryMessage::PrimaryAction))
        };
        let button_row = widget(HostStack::leading_row(6.0)).children((
            primary("小", ControlSize::Small, ButtonKind::Subtle, small),
            primary("中", ControlSize::Medium, ButtonKind::Primary, medium),
            primary("大", ControlSize::Large, ButtonKind::Subtle, large),
            widget(loading_button(state))
                .entity_ref(loading)
                .on(queue(pending, |_: &Activate| GalleryMessage::ToggleLoading)),
            widget(IconButton::new(Icon::Add, "添加").size(ControlSize::Small))
                .entity_ref(add)
                .on(queue(pending, |_: &Activate| GalleryMessage::PrimaryAction)),
        ));
        let segmented_row = widget(HostStack::leading_row(6.0)).children(
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
        let buttons_panel = widget(panel(6.0, Some(LengthSpec::Px(170.0)), 1.0)).children((
            widget(styled_text(
                "三档操作",
                SemanticColorRole::Muted,
                type_scale::META,
                type_scale::REGULAR,
            )),
            button_row,
            segmented_row,
            widget(styled_text(
                format!("主要操作已触发 {} 次", state.primary_clicks),
                SemanticColorRole::Faint,
                10.0,
                400,
            ))
            .entity_ref(clicks),
        ));

        let text_changed = || {
            queue(pending, |event: &TextChanged| {
                GalleryMessage::InputChanged(event.value.to_string())
            })
        };
        let input_row = widget(HostStack::fill_row(6.0)).children(
            ["小", "中", "大"]
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
        let dropdown_row = widget(HostStack::fill_row(6.0)).children(
            ["小", "中", "大"]
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
        let secure = inputs[3];
        let fields = widget(panel(5.0, Some(LengthSpec::Px(208.0)), 1.0)).children((
            widget(styled_text(
                "字段名称 *",
                SemanticColorRole::Text,
                13.0,
                600,
            )),
            input_row,
            widget(
                TextInput::new(state.input.clone())
                    .placeholder("配对密钥")
                    .secure(true),
            )
            .entity_ref(secure)
            .on(text_changed()),
            dropdown_row,
            widget(field_status_text(state)).entity_ref(field_status),
        ));

        let toggle_row =
            widget(HostStack::fill_row(8.0).align(nana_ui::runtime::AlignSpec::Center)).children((
                flex_cell(
                    widget(fill_range_field(
                        nana_ui::runtime::RangeField::new(f64::from(state.slider), 0.0, 100.0, 1.0)
                            .label("强度")
                            .unit("%"),
                    ))
                    .entity_ref(range)
                    .on(queue(pending, |event: &RangeInput| {
                        GalleryMessage::SetSlider(event.value.round() as u8)
                    })),
                ),
                widget(
                    HostStack::column(0.0)
                        .width(LengthSpec::Px(116.0))
                        .max_width(LengthSpec::Px(116.0))
                        .grow(0.0)
                        .shrink(0.0),
                )
                .children(widget(gallery_search(state)).entity_ref(search).on(
                    queue(pending, |event: &SearchDropdownEvent| {
                        map_search_event(event)
                    }),
                )),
            ));
        let toggles = widget(panel(8.0, Some(LengthSpec::Px(170.0)), 1.0)).children((
            widget(styled_text(
                "选择控件",
                SemanticColorRole::Muted,
                type_scale::META,
                type_scale::REGULAR,
            )),
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
            toggle_row,
        ));

        let text_area = widget(filling_panel(5.0)).children((
            widget(styled_text("多行文本", SemanticColorRole::Text, 13.0, 600)),
            widget(gallery_textarea(state))
                .entity_ref(textarea)
                .on(queue(pending, |event: &TextChanged| {
                    GalleryMessage::SetEditorText(event.value.to_string())
                })),
            widget(editor_status_text(state)).entity_ref(editor_status),
        ));
        let xy = widget(filling_panel(8.0)).children((
            widget(styled_text(
                "二维参数",
                SemanticColorRole::Muted,
                type_scale::META,
                type_scale::REGULAR,
            )),
            widget(XYPad::new(state.xy_pad).step(0.01))
                .entity_ref(xy_pad)
                .on(queue(pending, |event: &XYPadEvent| {
                    GalleryMessage::SetXYPad(*event)
                })),
            widget(styled_text(
                format!("X {:.2} · Y {:.2}", state.xy_pad.x, state.xy_pad.y),
                SemanticColorRole::Muted,
                type_scale::HINT,
                type_scale::REGULAR,
            ))
            .entity_ref(xy_label),
        ));

        let list = widget(
            HostStack::column(4.0)
                .height(LengthSpec::Fill)
                .min_width(LengthSpec::Px(0.0)),
        )
        .with(|list| {
            for (index, (label, disabled, size)) in LIST_ITEMS.into_iter().enumerate() {
                let selected = state.selected_item == index;
                let item = widget(list_item_spec(label, size, selected, disabled))
                    .entity_ref(list_items[index])
                    .leading(widget(list_leading_text(selected)).entity_ref(list_leads[index]))
                    .content(widget(list_label_text(label)).entity_ref(list_labels[index]))
                    .trailing(widget(list_trailing_text(disabled)).entity_ref(list_trails[index]));
                list.add(if disabled {
                    item
                } else {
                    item.on(queue(pending, move |_: &Activate| {
                        GalleryMessage::SelectListItem(index)
                    }))
                });
            }
        });
        let list_panel = widget(filling_panel(8.0)).children((
            widget(styled_text(
                "列表",
                SemanticColorRole::Muted,
                type_scale::META,
                type_scale::REGULAR,
            )),
            widget(HostStack::leading_row(8.0)).children((
                widget(Thumbnail::empty()),
                widget(Thumbnail::loading()),
                widget(Thumbnail::new("gallery.thumb")),
                widget(Thumbnail::unavailable()),
            )),
            widget(HostStack::leading_row(8.0)).children((
                widget(Chip::new("默认")),
                widget(Chip::new("已选").selected(true)),
            )),
            widget(HostStack::leading_row(8.0)).children((
                widget(Avatar::empty().label("空")),
                widget(Avatar::empty().size(40.0).label("大")),
            )),
            widget(ListItem::new("缩略图项"))
                .leading(widget(Thumbnail::empty()))
                .content(widget(list_label_text("缩略图项"))),
            list,
        ));

        let view = (
            widget(HostStack::canvas()).entity_ref(root).children((
                widget(HostStack::fill_row(10.0)).children((buttons_panel, fields, toggles)),
                widget(
                    HostStack::fill_row(10.0)
                        .height(LengthSpec::Fill)
                        .min_height(LengthSpec::Px(0.0))
                        .grow(1.0),
                )
                .children((text_area, xy, list_panel)),
            )),
            segmented_options,
        );
        with_refs(
            view,
            (
                (root, buttons, add, clicks),
                (segmented, segmented_on, segmented_off),
                (inputs, dropdowns, texts),
                (checkbox, switch, range, search, textarea, xy_pad),
                (list_items, list_leads, list_labels, list_trails),
            ),
        )
    })?;
    let (
        (root, [small, medium, large, loading], add, clicks),
        (segmented, segmented_on, segmented_off),
        (inputs, dropdowns, [field_status, editor_status, xy_label]),
        (checkbox, switch, range, search, textarea, xy_pad),
        (list_items, list_leads, list_labels, list_trails),
    ) = refs;
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
        _small: small,
        _medium: medium,
        _large: large,
        loading,
        _add: add,
        clicks,
        segmented,
        segmented_on,
        segmented_off,
        inputs: [input_small, input_medium, input_large],
        secure,
        dropdowns,
        field_status,
        checkbox,
        switch,
        range,
        search,
        textarea,
        editor_status,
        xy_pad,
        xy_label,
        list_items,
        list_leads,
        list_labels,
        list_trails,
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
        let root = entity_ref::<HostStack>();
        let tabs = entity_ref::<Tabs>();
        let surface_row = entity_ref::<HostStack>();
        let overview: [EntityRef<Card>; 3] = std::array::from_fn(|_| entity_ref());
        let cards: [EntityRef<InteractiveCard>; 3] = std::array::from_fn(|_| entity_ref());
        let tree = entity_ref::<TreeView>();
        let pane = entity_ref::<PaneChrome>();
        let pane_texts: [EntityRef<Text>; 5] = std::array::from_fn(|_| entity_ref());
        let [pane_tabs, pane_empty, pane_editor, pane_left, pane_right] = pane_texts;
        let pane_tree = entity_ref::<PaneTree>();
        let pane_split = entity_ref::<Button>();
        let pane_close = entity_ref::<IconButton>();
        let empty = entity_ref::<EmptyState>();
        let labeled = entity_ref::<LabeledValue>();

        let overview_data = [
            ("基础表面", "主工作区内容层", CardKind::Surface),
            ("抬升表面", "侧栏与工具面板", CardKind::Raised),
            ("选中表面", "当前激活的内容", CardKind::Selected),
        ];
        let overview_views = overview_data
            .into_iter()
            .zip(overview)
            .map(|((title, detail, kind), card)| {
                let mut card_view = Card::new().kind(kind).height(96.0).title(title);
                apply_equal_fill(std::sync::Arc::make_mut(&mut card_view.style.layout), 96.0);
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
        let cards_data = [
            ("默认卡片", "普通内容容器", false),
            ("交互卡片", "支持选择操作", false),
            ("禁用卡片", "不可进行操作", true),
        ];
        let card_views = cards_data
            .into_iter()
            .zip(cards)
            .enumerate()
            .map(|(index, ((title, detail, disabled), card))| {
                widget(
                    InteractiveCard::new()
                        .selected(state.selected_surface_card == index)
                        .disabled(disabled)
                        .style({
                            let mut style = nana_ui::runtime::NodeStyle::default();
                            apply_equal_fill(std::sync::Arc::make_mut(&mut style.layout), 96.0);
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

        let section_text = |value: &'static str, color, size| {
            widget(styled_text(value, color, size, type_scale::REGULAR))
        };
        let tab_bar = widget(HostStack::fill_row(8.0).align(nana_ui::runtime::AlignSpec::Center))
            .children((
                widget(hugging_text(
                    "表面状态",
                    SemanticColorRole::Text,
                    type_scale::META,
                    type_scale::REGULAR,
                )),
                widget(HostStack::spacer()),
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
            ));
        let content = widget(HostStack::canvas()).entity_ref(root).children((
            section_text("表面层级", SemanticColorRole::Text, type_scale::SECTION),
            section_text(
                "基础、抬升与选中状态",
                SemanticColorRole::Muted,
                type_scale::HINT,
            ),
            widget(panel(8.0, None, 0.0)).children(tab_bar),
            widget(HostStack::fill_row(10.0))
                .entity_ref(surface_row)
                .children(shown),
            section_text("层级树", SemanticColorRole::Text, type_scale::SECTION),
            section_text(
                "稳定节点 ID 驱动展开与选择",
                SemanticColorRole::Muted,
                type_scale::HINT,
            ),
            widget(panel(8.0, None, 0.0)).children(
                widget(gallery_tree(state)).entity_ref(tree).on(queue(
                    pending,
                    |event: &TreeViewEvent<Arc<str>>| match event {
                        TreeViewEvent::Toggle(id) => {
                            GalleryMessage::TreeView(TreeViewEvent::Toggle(id.to_string()))
                        }
                        TreeViewEvent::Select(id) => {
                            GalleryMessage::TreeView(TreeViewEvent::Select(id.to_string()))
                        }
                    },
                )),
            ),
            section_text("Pane 组合", SemanticColorRole::Text, type_scale::SECTION),
            section_text(
                "动作只在具备真实 handler 时出现",
                SemanticColorRole::Muted,
                type_scale::HINT,
            ),
            widget(panel(0.0, Some(LengthSpec::Px(140.0)), 0.0)).children(pane_view),
            widget(EmptyState::new("没有选中的表面").message("选择一张卡片查看详情"))
                .entity_ref(empty),
            widget(LabeledValue::new(
                "当前卡片",
                format!("{}", state.selected_surface_card),
            ))
            .entity_ref(labeled),
        ));
        let view = (content, hidden);
        with_refs(
            view,
            (
                (root, tabs, surface_row, overview, cards, tree, pane),
                pane_texts,
                (pane_tree, pane_split, pane_close, empty, labeled),
            ),
        )
    })?;
    let (
        (root, tabs, surface_row, overview, cards, tree, pane),
        [pane_tabs, pane_empty, pane_editor, pane_left, pane_right],
        (pane_tree, pane_split, pane_close, empty, labeled),
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
        pane,
        pane_tabs,
        pane_tree,
        pane_empty,
        pane_editor,
        pane_left,
        pane_right,
        pane_split,
        pane_close,
        _empty: empty,
        labeled,
    })
}

fn mount_feedback(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    state: &GalleryState,
    pending: &Arc<Mutex<Vec<GalleryMessage>>>,
) -> Result<FeedbackTree, FrameworkError> {
    let progress_value = if state.loading { 72.0 } else { 0.0 };
    let (_, refs) = context.mount_view_detached(document_id, || {
        let root = entity_ref::<HostStack>();
        let progress = entity_ref::<Progress>();
        let spinner = entity_ref::<Spinner>();
        let skeleton = entity_ref::<Skeleton>();
        let meter = entity_ref::<LevelMeter>();
        let badge = entity_ref::<StatusBadge>();
        let validation = entity_ref::<ValidationMessage>();
        let toast = entity_ref::<Toast>();
        let actions: [EntityRef<Button>; 4] = std::array::from_fn(|_| entity_ref());
        let [dialog, context_button, image, popover_action] = actions;
        let popover = entity_ref::<Popover>();
        let calendar = entity_ref::<CalendarHeatmap>();
        let calendar_status = entity_ref::<Text>();
        let action = entity_ref::<Text>();

        let progress_panel = widget(panel(8.0, Some(LengthSpec::Px(160.0)), 1.0)).children((
            widget(styled_text(
                if state.loading {
                    "处理中"
                } else {
                    "已完成"
                },
                SemanticColorRole::Text,
                13.0,
                400,
            )),
            widget(Progress::new(progress_value, 100.0)).entity_ref(progress),
            widget(Spinner::new(if state.loading {
                "处理中"
            } else {
                "已完成"
            }))
            .entity_ref(spinner),
            widget(Skeleton::fill_width(8.0)).entity_ref(skeleton),
            widget(LevelMeter::new(f32::from(state.slider) / 100.0)).entity_ref(meter),
        ));
        let action_panel = widget(
            HostStack::panel(8.0)
                .width(LengthSpec::Px(140.0))
                .max_width(LengthSpec::Px(140.0))
                .grow(0.0)
                .shrink(0.0),
        )
        .children((
            widget(fill_action_button(
                if state.overlay.contains(&super::GalleryOverlay::Dialog) {
                    "关闭对话框"
                } else {
                    "打开对话框"
                },
                ButtonKind::Primary,
            ))
            .entity_ref(dialog)
            .on(queue(pending, |_: &Activate| GalleryMessage::ToggleDialog)),
            widget(fill_action_button("打开更多操作", ButtonKind::Subtle))
                .entity_ref(context_button)
                .on(queue(pending, |_: &Activate| {
                    GalleryMessage::ToggleContextMenu
                })),
            widget(fill_action_button("查看图片", ButtonKind::Subtle))
                .entity_ref(image)
                .on(queue(pending, |_: &Activate| {
                    GalleryMessage::ToggleImageViewer
                })),
        ));
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
        let calendar_panel = widget(panel(6.0, None, 0.0)).children((
            widget(styled_text(
                "日历热力图",
                SemanticColorRole::Muted,
                12.0,
                400,
            )),
            widget(CalendarHeatmap::new(gallery_calendar_data())).entity_ref(calendar),
            widget(styled_text(
                state
                    .calendar_active
                    .as_ref()
                    .map_or("移动指针查看日期".to_owned(), |cell| {
                        cell.title.clone()
                    }),
                SemanticColorRole::Muted,
                10.0,
                400,
            ))
            .entity_ref(calendar_status),
        ));
        let view = widget(HostStack::canvas()).entity_ref(root).children((
            widget(styled_text(
                "反馈",
                SemanticColorRole::Text,
                type_scale::SECTION,
                type_scale::REGULAR,
            )),
            widget(HostStack::fill_row(10.0).align(nana_ui::runtime::AlignSpec::Start))
                .children((flex_cell(progress_panel), action_panel)),
            widget(
                HostStack::column(0.0)
                    .width(LengthSpec::Fill)
                    .padding_xy(0.0, 8.0)
                    .min_height(LengthSpec::Px(32.0))
                    .grow(0.0)
                    .shrink(0.0),
            )
            .children(popover_view),
            calendar_panel,
            widget(StatusBadge::new(action_status(state), StatusTone::Info)).entity_ref(badge),
            widget(ValidationMessage::new(
                "等待操作",
                ValidationIntent::Warning,
            ))
            .entity_ref(validation),
            widget(Toast::new(action_status(state), ToastTone::Info)).entity_ref(toast),
            widget(styled_text(
                action_status(state),
                SemanticColorRole::Muted,
                10.0,
                400,
            ))
            .entity_ref(action),
        ));
        with_refs(
            view,
            (
                (root, progress, spinner, skeleton, meter),
                (badge, validation, toast),
                actions,
                (popover, calendar, calendar_status, action),
            ),
        )
    })?;
    let (
        (root, progress, spinner, skeleton, meter),
        (badge, validation, toast),
        [dialog, context_button, image, popover_action],
        (popover, calendar, calendar_status, action),
    ) = refs;
    Ok(FeedbackTree {
        root,
        progress,
        spinner,
        _skeleton: skeleton,
        meter,
        badge,
        _validation: validation,
        toast,
        dialog,
        context: context_button,
        _image: image,
        popover,
        popover_action,
        calendar,
        calendar_status,
        action_status: action,
    })
}

fn mount_rich_text(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    state: &GalleryState,
    pending: &Arc<Mutex<Vec<GalleryMessage>>>,
) -> Result<RichTextMount, FrameworkError> {
    let (_, refs) = context.mount_view_detached(document_id, || {
        let stacks: [EntityRef<HostStack>; 2] = std::array::from_fn(|_| entity_ref());
        let [root, drop] = stacks;
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
        let title =
            |value: &'static str| widget(styled_text(value, SemanticColorRole::Text, 13.0, 600));
        let view = widget(HostStack::canvas()).entity_ref(root).children((
            widget(styled_text(
                "原生富文本",
                SemanticColorRole::Text,
                20.0,
                600,
            )),
            widget(styled_text(
                "CommonMark、数学公式与图表共享同一 Runtime Scene 渲染路径。",
                SemanticColorRole::Muted,
                12.0,
                400,
            )),
            widget(HostStack::row(24.0)).children((
                widget(vertical),
                widget(vertical_editor),
                widget(vertical_rtl),
            )),
            (
                widget(state.markdown.clone()).entity_ref(markdown),
                widget(styled_text(
                    state
                        .opened_markdown_link
                        .as_ref()
                        .map_or(String::new(), |link| format!("已选择链接：{link}")),
                    SemanticColorRole::Accent,
                    type_scale::HINT,
                    type_scale::REGULAR,
                ))
                .entity_ref(link_status),
                title("代码编辑器"),
                widget(gallery_code_editor(state)).entity_ref(editor),
                title("终端"),
                widget(gallery_terminal_view()).entity_ref(terminal),
                title("拖入文件"),
                widget(
                    HostStack::column(8.0)
                        .padding(12.0)
                        .background(SemanticColorRole::Subtle)
                        .min_height(LengthSpec::Px(48.0)),
                )
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
                title("差异"),
                widget(gallery_diff_view()).entity_ref(diff),
            ),
        ));
        with_refs(view, (stacks, markdown, texts, editor, terminal, diff))
    })?;
    let ([root, drop], markdown, [link_status, drop_hint], editor, terminal, diff) = refs;
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
        let view = widget(HostStack::canvas()).entity_ref(root).children((
            widget(HostStack::fill_row(10.0).align(nana_ui::runtime::AlignSpec::Center)).children(
                (
                    widget(hugging_text(
                        "节点图",
                        SemanticColorRole::Text,
                        type_scale::SECTION,
                        type_scale::REGULAR,
                    )),
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
                ),
            ),
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
        let popup = entity_ref::<AppShell>();

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
        let dock_frame = widget(
            HostStack::column(0.0)
                .height(LengthSpec::Px(WORKSPACE_DOCK_HEIGHT))
                .min_height(LengthSpec::Px(WORKSPACE_DOCK_HEIGHT))
                .grow(0.0)
                .shrink(0.0),
        )
        .children(
            widget(runtime_dock_from_workspace(
                state,
                &std::collections::HashMap::new(),
            ))
            .entity_ref(dock)
            .children(panel_views),
        );
        let body_view = widget(HostStack::column(4.0).padding(12.0).grow(0.0).shrink(0.0))
            .children((
                widget(styled_text(
                    "独立弹窗内容",
                    SemanticColorRole::Text,
                    13.0,
                    400,
                )),
                widget(styled_text(
                    "快速创建并管理项目",
                    SemanticColorRole::Muted,
                    type_scale::HINT,
                    type_scale::REGULAR,
                )),
            ));
        let frame_view = widget(
            HostStack::column(0.0)
                .width(LengthSpec::Px(WORKSPACE_POPUP_WIDTH))
                .height(LengthSpec::Px(WORKSPACE_POPUP_HEIGHT))
                .min_height(LengthSpec::Px(WORKSPACE_POPUP_HEIGHT))
                .background(SemanticColorRole::Surface)
                .grow(0.0)
                .shrink(0.0),
        )
        .children(
            widget(AppShell::new())
                .entity_ref(popup)
                .title_bar(widget(AppTitleBar::new("弹窗标题")))
                .body(body_view),
        );
        let view = widget(
            HostStack::fill_column(WORKSPACE_CANVAS_GAP)
                .padding(WORKSPACE_CANVAS_PADDING)
                .background(SemanticColorRole::Background)
                .grow(0.0),
        )
        .entity_ref(root)
        .children((tools, dock_frame, frame_view));
        with_refs(view, (root, dock, panels, buttons, status, popup))
    })?;
    let (root, dock, panels, [lock, hide, reset], status, popup) = refs;
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
        _popup: popup,
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
            widget(fill_range_field(
                nana_ui::runtime::RangeField::new(f64::from(radius), 0.0, 24.0, 1.0)
                    .label("标准圆角")
                    .unit("px"),
            ))
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
        let view = region_panel(
            pending,
            (8.0, 8.0),
            "底部面板",
            RegionId::Diagnostics,
            stacks,
            collapse,
            widget(StatusBadge::new("布局就绪", StatusTone::Success)),
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
        *label = styled_text(
            format!("主要操作已触发 {} 次", state.primary_clicks),
            SemanticColorRole::Faint,
            10.0,
            400,
        );
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
        let placeholder = ["小", "中", "大"][index];
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
    let _ = context.set_component(tree.checkbox, Checkbox::new("启用选项", state.checked));
    let _ = context.update_component(tree.switch, |switch, _| {
        *switch = Switch::new("允许编辑说明", state.switched).disabled(!state.checked);
    });
    let _ = context.update_component(tree.range, |range, _| {
        range.value = f64::from(state.slider);
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
        *label = styled_text(
            format!("X {:.2} · Y {:.2}", state.xy_pad.x, state.xy_pad.y),
            SemanticColorRole::Muted,
            type_scale::HINT,
            type_scale::REGULAR,
        );
    });
    for (index, item) in tree.list_items.iter().copied().enumerate() {
        let (label, disabled, size) = LIST_ITEMS[index];
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
            *mark = list_trailing_text(disabled);
        });
        let _ = context.update_component(item, |row, _| {
            *row = gallery_list_item(label, size, selected, disabled, leading, content, trailing);
        });
        let _ = context.set_list_item_slots(item, list_item_slots(leading, content, trailing));
    }
}

fn sync_surfaces(
    context: &mut nana_ui::runtime::AppContext,
    tree: &SurfacesTree,
    state: &GalleryState,
) {
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
        *value = LabeledValue::new("当前卡片", format!("{}", state.selected_surface_card));
    });
}

fn sync_feedback(
    context: &mut nana_ui::runtime::AppContext,
    tree: &FeedbackTree,
    state: &GalleryState,
) {
    let _ = context.update_component(tree.progress, |progress, _| {
        progress.value = if state.loading { 72.0 } else { 0.0 };
    });
    let _ = context.update_component(tree.spinner, |spinner, _| {
        *spinner = Spinner::new(if state.loading {
            "处理中"
        } else {
            "已完成"
        });
    });
    let _ = context.update_component(tree.meter, |meter, _| {
        meter.value = f32::from(state.slider) / 100.0;
    });
    let _ = context.update_component(tree.badge, |badge, _| {
        *badge = StatusBadge::new(action_status(state), StatusTone::Info);
    });
    let _ = context.update_component(tree.toast, |toast, _| {
        *toast = Toast::new(action_status(state), ToastTone::Info);
    });
    let _ = context.update_component(tree.dialog, |button, _| {
        *button = fill_action_button(
            if state.overlay.contains(&super::GalleryOverlay::Dialog) {
                "关闭对话框"
            } else {
                "打开对话框"
            },
            ButtonKind::Primary,
        );
    });
    let _ = context.update_component(tree.popover, |popover, _| {
        popover.open = state.popover_open;
    });
    let _ = context.update_component(tree.popover_action, |button, _| {
        *button = popover_action_button(state.popover_open);
    });
    let _ = context.update_component(tree.calendar_status, |label, _| {
        *label = styled_text(
            state
                .calendar_active
                .as_ref()
                .map_or("移动指针查看日期".to_owned(), |cell| {
                    cell.title.clone()
                }),
            SemanticColorRole::Muted,
            10.0,
            400,
        );
    });
    let _ = context.update_component(tree.action_status, |label, _| {
        *label = styled_text(
            action_status(state),
            SemanticColorRole::Muted,
            type_scale::HINT,
            type_scale::REGULAR,
        );
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

fn filling_panel(gap: f32) -> HostStack {
    HostStack::panel(gap)
        .height(LengthSpec::Fill)
        .min_height(LengthSpec::Px(0.0))
        .grow(1.0)
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
    layout.white_space_nowrap = true;
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

fn fill_action_button(label: impl Into<String>, kind: ButtonKind) -> Button {
    let mut button = Button::new(label).kind(kind);
    let layout = std::sync::Arc::make_mut(&mut button.style.layout);
    layout.width = Some(LengthSpec::Fill);
    layout.min_width = Some(LengthSpec::Px(0.0));
    layout.flex_grow = Some(0.0);
    layout.flex_shrink = Some(0.0);
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
    layout.white_space_nowrap = true;
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

fn list_trailing_text(disabled: bool) -> nana_ui::runtime::Text {
    list_slot_text(
        if disabled { "不可用" } else { "" },
        SemanticColorRole::Muted,
        type_scale::HINT,
        400,
        false,
    )
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
    size: ControlSize,
    selected: bool,
    disabled: bool,
    leading: Entity<nana_ui::runtime::Text>,
    content: Entity<nana_ui::runtime::Text>,
    trailing: Entity<nana_ui::runtime::Text>,
) -> ListItem {
    list_item_spec(label, size, selected, disabled)
        .slots(list_item_slots(leading, content, trailing))
}

/// A list row without its slots, which a view gives it as children.
fn list_item_spec(label: &str, size: ControlSize, selected: bool, disabled: bool) -> ListItem {
    let mut item = ListItem::new(label)
        .size(size)
        .selected(selected)
        .disabled(disabled);
    let layout = std::sync::Arc::make_mut(&mut item.style.layout);
    layout.width = Some(LengthSpec::Fill);
    layout.min_width = Some(LengthSpec::Px(0.0));
    layout.flex_grow = Some(0.0);
    layout.flex_shrink = Some(0.0);
    layout.white_space_nowrap = true;
    layout.text_overflow_ellipsis = true;
    item
}

fn panel(gap: f32, height: Option<LengthSpec>, grow: f32) -> HostStack {
    let mut stack = HostStack::panel(gap)
        .grow(grow)
        .min_width(LengthSpec::Px(0.0));
    if let Some(height) = height {
        stack = stack.height(height);
    }
    stack
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
                TreeNode::leaf(Arc::<str>::from("src/lib.rs"), "lib.rs")
                    .icon(Icon::File)
                    .selected(state.tree_selected == "src/lib.rs"),
                TreeNode::leaf(Arc::<str>::from("src/main.rs"), "main.rs")
                    .icon(Icon::File)
                    .selected(state.tree_selected == "src/main.rs"),
            ],
        )
        .icon(Icon::Folder)
        .selected(state.tree_selected == "src"),
        TreeNode::leaf(Arc::<str>::from("README.md"), "README.md")
            .icon(Icon::File)
            .selected(state.tree_selected == "README.md"),
    ])
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
