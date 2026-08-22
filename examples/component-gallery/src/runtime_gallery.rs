use std::fmt;
use std::sync::{Arc, Mutex};

use nana_ui::runtime::{
    Activate, AppShell, AppTitleBar, Button, CalendarHeatmap, CalendarHeatmapDatum,
    CalendarHeatmapEvent, Card, Checkbox, DesktopShell, DockFloatingSurface, DocumentId, Dropdown,
    DropdownEvent, DropdownOption, EmptyState, Entity, FrameworkError, GraphCanvas,
    GraphCanvasEvent, IconButton, InteractiveCard, LabeledValue, LayoutViewport, LengthSpec,
    LevelMeter, ListItem, ListItemSlots, NativeMarkdown, OverlayHost, PaneChrome, PaneChromeAction,
    PaneChromeActionKind, PaneTree, PaneTreeNode, Popover, PopoverClosed, PopoverToggled, Progress,
    RangeChanged, RichTextEvent, RuntimeDocument, SearchDropdown, SearchDropdownEvent,
    SearchDropdownOption, SegmentedControl, SegmentedOption, SegmentedSelectionRequested,
    SemanticColorRole, SidebarFooter, SidebarFooterButton, SidebarFrame, SidebarRow,
    SidebarRowIcon, SidebarRowState, SidebarSection, Skeleton, Spinner, StableNodeId, StatusBadge,
    Switch, TabOption, Tabs, TabsEvent, TextArea, TextChanged, TextInput, Thumbnail, Toast,
    ToggleChanged, TreeNode, TreeView, TreeViewEvent, ValidationMessage,
    View, XYPad, XYPadEvent,
};
use nana_ui::{
    ButtonKind, CardKind, ControlSize, Icon, LogicalPoint, NanaTextShaper, RegionId,
    RuntimeInputAdapter, StatusTone, ToastTone, ValidationIntent, WorkspaceAction,
};
use nana_ui_platform::InputEvent;

use super::runtime_host::{
    DEFAULT_VIEWPORT, HostStack, RuntimeChrome, RuntimeSceneInput, apply_title_bar_insets,
    apply_workspace_corners, bind_event, hugging_text, labeled_text, node_is_or_under,
    reconcile_children, runtime_input_event, search_command_button, sidebar_toggle_button,
    styled_text, take_pending, theme_toggle_button,
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
);
type GraphMount = (
    Entity<HostStack>,
    Entity<GraphCanvas>,
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
    graph_root: Entity<HostStack>,
    graph: Entity<GraphCanvas>,
    graph_selection: Entity<nana_ui::runtime::Text>,
    _graph_reset: Entity<Button>,
    workspace: WorkspaceTree,
    inspector: InspectorTree,
    _bottom_collapse: Entity<Button>,
    _toolbar_reset: Entity<Button>,
    last_viewport: LayoutViewport,
    chrome: RuntimeChrome,
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
        let (rich_text_root, rich_text, link_status) =
            mount_rich_text(context, document_id, state, &pending)?;
        let (graph_root, graph, graph_selection, graph_reset) =
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
        let title_leading =
            context.create_detached_component(document_id, HostStack::leading_row(0.0))?;
        let mut sidebar_toggle = None;
        context.mount(title_leading, |ui| {
            sidebar_toggle = Some(ui.child("toggle", sidebar_toggle_button(sidebar_collapsed))?);
            Ok(())
        })?;
        let sidebar_toggle = sidebar_toggle.expect("sidebar toggle");
        let search_button =
            context.create_detached_component(document_id, search_command_button())?;
        let theme_button =
            context.create_detached_component(document_id, theme_toggle_button(state.theme))?;
        let context_label = context.create_detached_component(
            document_id,
            hugging_text(
                section_label(state.section),
                SemanticColorRole::Muted,
                11.0,
                400,
            ),
        )?;
        let title_center = context.create_detached_component(
            document_id,
            hugging_text("NanaUI Gallery", SemanticColorRole::Text, 13.0, 600),
        )?;
        let title_trailing = context.create_detached_component(document_id, HostStack::row(6.0))?;
        context.append_child(title_trailing, context_label)?;
        context.append_child(title_trailing, search_button)?;
        context.append_child(title_trailing, theme_button)?;

        let primary = section_root(
            state.section,
            &controls,
            &surfaces,
            &feedback,
            rich_text_root,
            graph_root,
            &workspace,
        );
        let shell = context.create_component(
            document_id,
            DesktopShell::from_model(state.workspace.model().clone())
                .title("NanaUI Gallery")
                .title_leading(title_leading.stable_id())
                .title_center(title_center.stable_id())
                .title_trailing(title_trailing.stable_id())
                .navigation(sidebar.stable_id())
                .primary(primary)
                .inspector(inspector.slot.stable_id())
                .bottom(bottom.stable_id())
                .region(RegionId::PrimaryToolbar, toolbar.stable_id()),
        )?;
        context.assemble_desktop_shell(shell)?;
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
            graph_root,
            graph,
            graph_selection,
            _graph_reset: graph_reset,
            workspace,
            inspector,
            _bottom_collapse: bottom_collapse,
            _toolbar_reset: toolbar_reset,
            last_viewport,
            chrome: RuntimeChrome::default(),
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
                11.0,
                400,
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
                    11.0,
                    400,
                ),
                None => styled_text("", SemanticColorRole::Muted, 11.0, 400),
            };
        });
        let _ = context.update_component(self.graph, |canvas, _| {
            canvas.set_model(state.graph.clone());
            canvas.set_viewport(state.graph_viewport);
            canvas.set_selection(state.graph_selection.clone());
        });
        let _ = context.update_component(self.graph_selection, |label, _| {
            *label = hugging_text(
                graph_selection_label(state),
                SemanticColorRole::Muted,
                11.0,
                400,
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
        let chrome = state.window_chrome.chrome();
        apply_title_bar_insets(
            context,
            self.shell,
            chrome.leading_inset,
            chrome.trailing_inset,
            state.window_chrome.is_maximized(),
            chrome.uses_custom_controls(),
        );
        self.flush(state.gallery_viewport_size());
    }

    fn flush(&mut self, (width, height): (f32, f32)) {
        self.last_viewport = LayoutViewport::new(width, height);
        let _ = self.document.flush(self.last_viewport, &mut self.text);
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

    pub(super) fn flush_viewport(&mut self, size: (f32, f32)) {
        self.flush(size);
    }

    pub(super) fn note_pointer(&mut self, event: &InputEvent) {
        if let InputEvent::Pointer { x, y, .. } | InputEvent::Wheel { x, y, .. } = *event {
            self.chrome.last_pointer = LogicalPoint::new(x, y);
        }
    }

    pub(super) fn take_host_messages(&mut self, event: &InputEvent) -> Vec<GalleryMessage> {
        self.note_pointer(event);
        let extra = self.host_pointer_messages(event);
        let mut messages = take_pending(&self.pending);
        messages.extend(extra);
        messages.extend(self.chrome.workspace_resize_messages(&self.document, event));
        messages.extend(self.chrome.title_bar_chrome_messages(&self.document, event));
        messages
    }

    fn dispatch(&mut self, event: InputEvent) -> Vec<GalleryMessage> {
        if let InputEvent::Pointer { x, y, .. } | InputEvent::Wheel { x, y, .. } = event {
            self.chrome.last_pointer = LogicalPoint::new(x, y);
        }
        let extra = self.host_pointer_messages(&event);
        let document = self.document.document();
        let _ =
            RuntimeInputAdapter::default().dispatch(self.document.context_mut(), document, &event);
        let mut messages = take_pending(&self.pending);
        messages.extend(extra);
        messages.extend(
            self.chrome
                .workspace_resize_messages(&self.document, &event),
        );
        messages.extend(
            self.chrome
                .title_bar_chrome_messages(&self.document, &event),
        );
        messages
    }

    fn host_pointer_messages(&mut self, event: &InputEvent) -> Vec<GalleryMessage> {
        let InputEvent::Pointer {
            phase,
            button,
            x,
            y,
            ..
        } = *event
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
    fn first_dock_handle_drag(&self) -> Option<(LogicalPoint, LogicalPoint)> {
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
                let start = LogicalPoint::new(
                    bounds.x + bounds.width / 2.0,
                    bounds.y + bounds.height / 2.0,
                );
                let end = if bounds.width <= bounds.height {
                    LogicalPoint::new(start.x + 40.0, start.y)
                } else {
                    LogicalPoint::new(start.x, start.y + 40.0)
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
                Err(_) => return,
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
    pub(crate) fn gallery_runtime_dock_handle_drag(&self) -> Option<(LogicalPoint, LogicalPoint)> {
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
        let mut panels = Vec::new();
        let mut contents = std::collections::HashMap::new();
        for id in surface.root.flatten() {
            let (title, hint) = DOCK_PANELS
                .iter()
                .find(|(panel, _, _)| *panel == id.as_ref())
                .map(|(_, title, hint)| (*title, *hint))
                .unwrap_or(("Panel", ""));
            let panel = context.create_detached_component(
                document_id,
                HostStack::fill_column(5.0).padding(10.0),
            )?;
            let heading = context.create_detached_component(
                document_id,
                styled_text(title, SemanticColorRole::Text, 12.0, 400),
            )?;
            let detail = context.create_detached_component(
                document_id,
                styled_text(hint, SemanticColorRole::Muted, 10.0, 400),
            )?;
            context.append_child(panel, heading)?;
            context.append_child(panel, detail)?;
            contents.insert(id.to_string(), panel.stable_id());
            panels.push((id.to_string(), panel));
        }
        let dock = context.create_component(
            document_id,
            runtime_dock_from_node(state, &surface.root, &contents),
        )?;
        for (_, panel) in &panels {
            context.append_child(dock, *panel)?;
        }
        context.assemble_dock(dock)?;
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
        let _ = self.document.flush(
            LayoutViewport::new(surface.width.max(1.0), surface.height.max(1.0)),
            &mut self.text,
        );
    }

    pub(super) fn runtime_document(&self) -> &RuntimeDocument {
        &self.document
    }

    pub(super) fn runtime_document_mut(&mut self) -> &mut RuntimeDocument {
        &mut self.document
    }

    pub(super) fn resize(&mut self, width: f32, height: f32) {
        let _ = self.document.flush(
            LayoutViewport::new(width.max(1.0), height.max(1.0)),
            &mut self.text,
        );
    }
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
    let mut spec = SidebarSection::new("Gallery").count(6);
    let title = context.create_detached_component(document_id, spec.title_label())?;
    spec = spec.title_slot(title.stable_id());
    let header = context.create_detached_component(document_id, spec.header_item())?;
    context.append_child(header, title)?;
    let body = context.create_detached_component(document_id, SidebarSection::body_port())?;
    let mut rows = Vec::with_capacity(6);
    for (target, label, icon) in SECTIONS {
        let leading = context.create_detached_component(document_id, SidebarRowIcon::new(icon))?;
        let row = context.create_detached_component(
            document_id,
            SidebarRow::new(label)
                .state(if state.section == target {
                    SidebarRowState::Active
                } else {
                    SidebarRowState::Idle
                })
                .slots(ListItemSlots {
                    leading: Some(leading.stable_id()),
                    content: None,
                    trailing: None,
                }),
        )?;
        context.append_child(row, leading)?;
        context.append_child(body, row)?;
        rows.push(row);
    }
    let section = context.create_detached_component(
        document_id,
        spec.header(header.stable_id()).body(body.stable_id()),
    )?;
    context.append_child(section, header)?;
    context.append_child(section, body)?;
    let scroll =
        context.create_detached_component(document_id, SidebarFrame::vertical_body_scroll())?;
    context.append_child(scroll, section)?;
    let footer = context.create_detached_component(document_id, SidebarFooter::new())?;
    let settings = context.create_detached_component(
        document_id,
        SidebarFooterButton::new("设置", Icon::Settings),
    )?;
    context.append_child(footer, settings)?;
    let frame = context.create_detached_component(
        document_id,
        SidebarFrame::new()
            .body(scroll.stable_id())
            .footer(footer.stable_id()),
    )?;
    context.append_child(frame, scroll)?;
    context.append_child(frame, footer)?;
    let rows = [rows[0], rows[1], rows[2], rows[3], rows[4], rows[5]];
    Ok((frame, rows, settings))
}

fn mount_controls(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    state: &GalleryState,
    pending: &Arc<Mutex<Vec<GalleryMessage>>>,
) -> Result<ControlsTree, FrameworkError> {
    let small = context.create_detached_component(
        document_id,
        Button::new("小")
            .size(ControlSize::Small)
            .kind(ButtonKind::Subtle),
    )?;
    let medium = context.create_detached_component(
        document_id,
        Button::new("中")
            .size(ControlSize::Medium)
            .kind(ButtonKind::Primary),
    )?;
    let large = context.create_detached_component(
        document_id,
        Button::new("大")
            .size(ControlSize::Large)
            .kind(ButtonKind::Subtle),
    )?;
    let loading = context.create_detached_component(document_id, loading_button(state))?;
    let add = context.create_detached_component(
        document_id,
        IconButton::new(Icon::Add, "添加").size(ControlSize::Small),
    )?;
    let clicks = context.create_detached_component(
        document_id,
        styled_text(
            format!("主要操作已触发 {} 次", state.primary_clicks),
            SemanticColorRole::Faint,
            10.0,
            400,
        ),
    )?;
    for button in [small, medium, large] {
        bind_event(context, button, Arc::clone(pending), |event: &Activate| {
            let _ = event;
            GalleryMessage::PrimaryAction
        })?;
    }
    bind_event(context, add, Arc::clone(pending), |event: &Activate| {
        let _ = event;
        GalleryMessage::PrimaryAction
    })?;
    bind_event(context, loading, Arc::clone(pending), |event: &Activate| {
        let _ = event;
        GalleryMessage::ToggleLoading
    })?;

    let sizes = [ControlSize::Small, ControlSize::Medium, ControlSize::Large];
    let mut segmented = [None; 3];
    let mut segmented_on = [None; 3];
    let mut segmented_off = [None; 3];
    for (index, size) in sizes.iter().copied().enumerate() {
        let control =
            context.create_detached_component(document_id, SegmentedControl::new().size(size))?;
        let off = context
            .create_detached_component(document_id, SegmentedOption::new("关").size(size))?;
        let on = context
            .create_detached_component(document_id, SegmentedOption::new("开").size(size))?;
        context.set_segmented_options(
            control,
            vec![off, on],
            Some(if state.checked { on } else { off }),
        )?;
        bind_event(
            context,
            control,
            Arc::clone(pending),
            move |event: &SegmentedSelectionRequested| {
                GalleryMessage::ToggleCheck(event.option == on.stable_id())
            },
        )?;
        segmented[index] = Some(control);
        segmented_on[index] = Some(on);
        segmented_off[index] = Some(off);
    }

    let mut inputs = [None; 3];
    for (index, (placeholder, size)) in ["小", "中", "大"].iter().zip(sizes).enumerate() {
        let input = context.create_detached_component(
            document_id,
            TextInput::new(state.input.clone())
                .placeholder(*placeholder)
                .size(size)
                .invalid(state.input.trim().is_empty()),
        )?;
        bind_event(
            context,
            input,
            Arc::clone(pending),
            |event: &TextChanged| GalleryMessage::InputChanged(event.value.clone()),
        )?;
        inputs[index] = Some(input);
    }
    let secure = context.create_detached_component(
        document_id,
        TextInput::new(state.input.clone())
            .placeholder("配对密钥")
            .secure(true),
    )?;
    bind_event(
        context,
        secure,
        Arc::clone(pending),
        |event: &TextChanged| GalleryMessage::InputChanged(event.value.clone()),
    )?;

    let mut dropdowns = [None; 3];
    for (index, (placeholder, size)) in ["小", "中", "大"].iter().zip(sizes).enumerate() {
        let dropdown = context
            .create_detached_component(document_id, gallery_dropdown(state, placeholder, size))?;
        bind_event(
            context,
            dropdown,
            Arc::clone(pending),
            |event: &DropdownEvent<Arc<str>>| map_dropdown_event(event),
        )?;
        dropdowns[index] = Some(dropdown);
    }
    let field_status = context.create_detached_component(document_id, field_status_text(state))?;

    let checkbox =
        context.create_detached_component(document_id, Checkbox::new("启用选项", state.checked))?;
    bind_event(
        context,
        checkbox,
        Arc::clone(pending),
        |event: &ToggleChanged| GalleryMessage::ToggleCheck(event.checked),
    )?;
    let switch = context.create_detached_component(
        document_id,
        Switch::new("允许编辑说明", state.switched).disabled(!state.checked),
    )?;
    bind_event(
        context,
        switch,
        Arc::clone(pending),
        |event: &ToggleChanged| GalleryMessage::ToggleSwitch(event.checked),
    )?;
    let range = context.create_detached_component(
        document_id,
        fill_range_field(
            nana_ui::runtime::RangeField::new(f64::from(state.slider), 0.0, 100.0, 1.0)
                .expect("gallery range")
                .label("强度")
                .unit("%"),
        ),
    )?;
    bind_event(
        context,
        range,
        Arc::clone(pending),
        |event: &RangeChanged| GalleryMessage::SetSlider(event.value.round() as u8),
    )?;
    let search = context.create_detached_component(document_id, gallery_search(state))?;
    bind_event(
        context,
        search,
        Arc::clone(pending),
        |event: &SearchDropdownEvent| map_search_event(event),
    )?;

    let textarea = context.create_detached_component(document_id, gallery_textarea(state))?;
    bind_event(
        context,
        textarea,
        Arc::clone(pending),
        |event: &TextChanged| GalleryMessage::SetEditorText(event.value.clone()),
    )?;
    let editor_status =
        context.create_detached_component(document_id, editor_status_text(state))?;
    let xy_pad =
        context.create_detached_component(document_id, XYPad::new(state.xy_pad).step(0.01))?;
    bind_event(
        context,
        xy_pad,
        Arc::clone(pending),
        |event: &XYPadEvent| GalleryMessage::SetXYPad(*event),
    )?;
    let xy_label = context.create_detached_component(
        document_id,
        styled_text(
            format!("X {:.2} · Y {:.2}", state.xy_pad.x, state.xy_pad.y),
            SemanticColorRole::Muted,
            11.0,
            400,
        ),
    )?;

    let mut list_items = Vec::new();
    let mut list_leads = Vec::new();
    let mut list_labels = Vec::new();
    let mut list_trails = Vec::new();
    let list = context.create_detached_component(
        document_id,
        HostStack::column(4.0)
            .height(LengthSpec::Fill)
            .min_width(LengthSpec::Px(0.0)),
    )?;
    for (index, (label, disabled, size)) in LIST_ITEMS.into_iter().enumerate() {
        let selected = state.selected_item == index;
        let leading =
            context.create_detached_component(document_id, list_leading_text(selected))?;
        let content = context.create_detached_component(document_id, list_label_text(label))?;
        let trailing =
            context.create_detached_component(document_id, list_trailing_text(disabled))?;
        let item = context.create_detached_component(
            document_id,
            gallery_list_item(label, size, selected, disabled, leading, content, trailing),
        )?;
        context.append_child(item, leading)?;
        context.append_child(item, content)?;
        context.append_child(item, trailing)?;
        context.set_list_item_slots(item, list_item_slots(leading, content, trailing))?;
        if !disabled {
            bind_event(
                context,
                item,
                Arc::clone(pending),
                move |event: &Activate| {
                    let _ = event;
                    GalleryMessage::SelectListItem(index)
                },
            )?;
        }
        context.append_child(list, item)?;
        list_items.push(item);
        list_leads.push(leading);
        list_labels.push(content);
        list_trails.push(trailing);
    }

    let buttons_title = context.create_detached_component(
        document_id,
        styled_text("三档操作", SemanticColorRole::Muted, 12.0, 400),
    )?;
    let buttons = panel(
        context,
        document_id,
        6.0,
        Some(LengthSpec::Px(170.0)),
        &[buttons_title.stable_id()],
        1.0,
    )?;
    let button_row = context.create_detached_component(document_id, HostStack::leading_row(6.0))?;
    context.append_child(button_row, small)?;
    context.append_child(button_row, medium)?;
    context.append_child(button_row, large)?;
    context.append_child(button_row, loading)?;
    context.append_child(button_row, add)?;
    context.append_child(buttons, button_row)?;
    let segmented_row =
        context.create_detached_component(document_id, HostStack::leading_row(6.0))?;
    for control in segmented.iter().flatten().copied() {
        context.append_child(segmented_row, control)?;
    }
    context.append_child(buttons, segmented_row)?;
    context.append_child(buttons, clicks)?;

    let fields = panel(
        context,
        document_id,
        5.0,
        Some(LengthSpec::Px(208.0)),
        &[],
        1.0,
    )?;
    let name = context.create_detached_component(
        document_id,
        styled_text("字段名称 *", SemanticColorRole::Text, 13.0, 600),
    )?;
    context.append_child(fields, name)?;
    let input_row = context.create_detached_component(document_id, HostStack::fill_row(6.0))?;
    for input in inputs.iter().flatten().copied() {
        append_flex_child(context, document_id, input_row, input)?;
    }
    context.append_child(fields, input_row)?;
    context.append_child(fields, secure)?;
    let dropdown_row = context.create_detached_component(document_id, HostStack::fill_row(6.0))?;
    for dropdown in dropdowns.iter().flatten().copied() {
        append_flex_child(context, document_id, dropdown_row, dropdown)?;
    }
    context.append_child(fields, dropdown_row)?;
    context.append_child(fields, field_status)?;

    let toggles = panel(
        context,
        document_id,
        8.0,
        Some(LengthSpec::Px(170.0)),
        &[],
        1.0,
    )?;
    let toggle_title = context.create_detached_component(
        document_id,
        styled_text("选择控件", SemanticColorRole::Muted, 12.0, 400),
    )?;
    context.append_child(toggles, toggle_title)?;
    context.append_child(toggles, checkbox)?;
    context.append_child(toggles, switch)?;
    let toggle_row = context.create_detached_component(
        document_id,
        HostStack::fill_row(8.0).align(nana_ui::runtime::AlignSpec::Center),
    )?;
    append_flex_child(context, document_id, toggle_row, range)?;
    let search_cell = context.create_detached_component(
        document_id,
        HostStack::column(0.0)
            .width(LengthSpec::Px(116.0))
            .max_width(LengthSpec::Px(116.0))
            .grow(0.0)
            .shrink(0.0),
    )?;
    context.append_child(search_cell, search)?;
    context.append_child(toggle_row, search_cell)?;
    context.append_child(toggles, toggle_row)?;

    let text_area = filling_panel(context, document_id, 5.0)?;
    let editor_title = context.create_detached_component(
        document_id,
        styled_text("多行文本", SemanticColorRole::Text, 13.0, 600),
    )?;
    context.append_child(text_area, editor_title)?;
    context.append_child(text_area, textarea)?;
    context.append_child(text_area, editor_status)?;

    let xy = filling_panel(context, document_id, 8.0)?;
    let xy_title = context.create_detached_component(
        document_id,
        styled_text("二维参数", SemanticColorRole::Muted, 12.0, 400),
    )?;
    context.append_child(xy, xy_title)?;
    context.append_child(xy, xy_pad)?;
    context.append_child(xy, xy_label)?;

    let list_panel = filling_panel(context, document_id, 8.0)?;
    let list_title = context.create_detached_component(
        document_id,
        styled_text("列表", SemanticColorRole::Muted, 12.0, 400),
    )?;
    context.append_child(list_panel, list_title)?;
    let thumb_row =
        context.create_detached_component(document_id, HostStack::leading_row(8.0))?;
    for thumb in [
        Thumbnail::empty(),
        Thumbnail::loading(),
        Thumbnail::new("gallery.thumb"),
        Thumbnail::unavailable(),
    ] {
        let node = context.create_detached_component(document_id, thumb)?;
        context.append_child(thumb_row, node)?;
    }
    context.append_child(list_panel, thumb_row)?;
    let thumb_lead =
        context.create_detached_component(document_id, Thumbnail::empty())?;
    let thumb_label = context.create_detached_component(document_id, list_label_text("缩略图项"))?;
    let thumb_item = context.create_detached_component(
        document_id,
        ListItem::new("缩略图项").slots(ListItemSlots {
            leading: Some(thumb_lead.stable_id()),
            content: Some(thumb_label.stable_id()),
            trailing: None,
        }),
    )?;
    context.append_child(thumb_item, thumb_lead)?;
    context.append_child(thumb_item, thumb_label)?;
    context.set_list_item_slots(
        thumb_item,
        ListItemSlots {
            leading: Some(thumb_lead.stable_id()),
            content: Some(thumb_label.stable_id()),
            trailing: None,
        },
    )?;
    context.append_child(list_panel, thumb_item)?;
    context.append_child(list_panel, list)?;

    let top = context.create_detached_component(document_id, HostStack::fill_row(10.0))?;
    context.append_child(top, buttons)?;
    context.append_child(top, fields)?;
    context.append_child(top, toggles)?;
    let bottom = context.create_detached_component(
        document_id,
        HostStack::fill_row(10.0)
            .height(LengthSpec::Fill)
            .min_height(LengthSpec::Px(0.0))
            .grow(1.0),
    )?;
    context.append_child(bottom, text_area)?;
    context.append_child(bottom, xy)?;
    context.append_child(bottom, list_panel)?;
    let root = context.create_detached_component(document_id, HostStack::canvas())?;
    context.append_child(root, top)?;
    context.append_child(root, bottom)?;

    Ok(ControlsTree {
        root,
        _small: small,
        _medium: medium,
        _large: large,
        loading,
        _add: add,
        clicks,
        segmented: [
            segmented[0].expect("segmented"),
            segmented[1].expect("segmented"),
            segmented[2].expect("segmented"),
        ],
        segmented_on: [
            segmented_on[0].expect("on"),
            segmented_on[1].expect("on"),
            segmented_on[2].expect("on"),
        ],
        segmented_off: [
            segmented_off[0].expect("off"),
            segmented_off[1].expect("off"),
            segmented_off[2].expect("off"),
        ],
        inputs: [
            inputs[0].expect("input"),
            inputs[1].expect("input"),
            inputs[2].expect("input"),
        ],
        secure,
        dropdowns: [
            dropdowns[0].expect("dropdown"),
            dropdowns[1].expect("dropdown"),
            dropdowns[2].expect("dropdown"),
        ],
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
    let tabs = context.create_detached_component(
        document_id,
        Tabs::new(if selected == SurfaceView::Cards {
            "cards"
        } else {
            "overview"
        })
        .options([
            TabOption::new("overview", "概览"),
            TabOption::new("cards", "卡片"),
        ]),
    )?;
    bind_event(
        context,
        tabs,
        Arc::clone(pending),
        |event: &TabsEvent| match event {
            TabsEvent::Select(value) if value.as_ref() == "cards" => {
                GalleryMessage::SelectSurfaceView(SurfaceView::Cards)
            }
            TabsEvent::Select(_) => GalleryMessage::SelectSurfaceView(SurfaceView::Overview),
            _ => GalleryMessage::OverlayInteraction,
        },
    )?;

    let overview_data = [
        ("基础表面", "主工作区内容层", CardKind::Surface),
        ("抬升表面", "侧栏与工具面板", CardKind::Raised),
        ("选中表面", "当前激活的内容", CardKind::Selected),
    ];
    let mut overview = [None; 3];
    for (index, (title, detail, kind)) in overview_data.into_iter().enumerate() {
        let mut card_view = Card::new().kind(kind).height(96.0).title(title);
        apply_equal_fill(std::sync::Arc::make_mut(&mut card_view.style.layout), 96.0);
        let card = context.create_detached_component(document_id, card_view)?;
        let hint = context.create_detached_component(
            document_id,
            styled_text(detail, SemanticColorRole::Muted, 11.0, 400),
        )?;
        context.append_child(card, hint)?;
        overview[index] = Some(card);
    }
    let cards_data = [
        ("默认卡片", "普通内容容器", false),
        ("交互卡片", "支持选择操作", false),
        ("禁用卡片", "不可进行操作", true),
    ];
    let mut cards = [None; 3];
    for (index, (title, detail, disabled)) in cards_data.into_iter().enumerate() {
        let card = context.create_detached_component(
            document_id,
            InteractiveCard::new()
                .selected(state.selected_surface_card == index)
                .disabled(disabled)
                .style({
                    let mut style = nana_ui::runtime::NodeStyle::default();
                    apply_equal_fill(std::sync::Arc::make_mut(&mut style.layout), 96.0);
                    style
                }),
        )?;
        let heading = context.create_detached_component(
            document_id,
            styled_text(title, SemanticColorRole::Text, 13.0, 400),
        )?;
        let hint = context.create_detached_component(
            document_id,
            styled_text(detail, SemanticColorRole::Muted, 11.0, 400),
        )?;
        context.append_child(card, heading)?;
        context.append_child(card, hint)?;
        cards[index] = Some(card);
    }
    let surface_row = context.create_detached_component(document_id, HostStack::fill_row(10.0))?;
    if selected == SurfaceView::Cards {
        for card in cards.iter().flatten().copied() {
            context.append_child(surface_row, card)?;
        }
    } else {
        for card in overview.iter().flatten().copied() {
            context.append_child(surface_row, card)?;
        }
    }

    let tree = context.create_detached_component(document_id, gallery_tree(state))?;
    bind_event(
        context,
        tree,
        Arc::clone(pending),
        |event: &TreeViewEvent<Arc<str>>| match event {
            TreeViewEvent::Toggle(id) => {
                GalleryMessage::TreeView(TreeViewEvent::Toggle(id.to_string()))
            }
            TreeViewEvent::Select(id) => {
                GalleryMessage::TreeView(TreeViewEvent::Select(id.to_string()))
            }
        },
    )?;

    let pane_tabs = context.create_detached_component(
        document_id,
        hugging_text(
            if state.pane_chrome_item_open {
                "main.rs"
            } else {
                "空窗格"
            },
            SemanticColorRole::Text,
            11.0,
            400,
        ),
    )?;
    let pane_empty = context.create_detached_component(
        document_id,
        hugging_text("Item 已关闭", SemanticColorRole::Muted, 11.0, 400),
    )?;
    let pane_editor = context.create_detached_component(
        document_id,
        hugging_text("编辑器内容", SemanticColorRole::Text, 11.0, 400),
    )?;
    let pane_left = context.create_detached_component(
        document_id,
        hugging_text("左侧编辑器", SemanticColorRole::Text, 11.0, 400),
    )?;
    let pane_right = context.create_detached_component(
        document_id,
        hugging_text("右侧编辑器", SemanticColorRole::Text, 11.0, 400),
    )?;
    let pane_tree = context.create_detached_component(
        document_id,
        PaneTree::new(pane_tree_node(
            state,
            pane_empty,
            pane_editor,
            pane_left,
            pane_right,
        )),
    )?;
    reconcile_children(
        context,
        pane_tree.stable_id(),
        &pane_tree_children(state, pane_empty, pane_editor, pane_left, pane_right),
    )?;
    let pane_split = context.create_detached_component(
        document_id,
        Button::new("左右分栏")
            .kind(ButtonKind::Text)
            .size(ControlSize::Small),
    )?;
    bind_event(
        context,
        pane_split,
        Arc::clone(pending),
        |event: &Activate| {
            let _ = event;
            GalleryMessage::PaneChrome(PaneChromeActionKind::SplitHorizontal)
        },
    )?;
    let pane_close = context.create_detached_component(
        document_id,
        IconButton::new(Icon::Close, "关闭 Item")
            .size(ControlSize::Small)
            .kind(ButtonKind::Text),
    )?;
    bind_event(
        context,
        pane_close,
        Arc::clone(pending),
        |event: &Activate| {
            let _ = event;
            GalleryMessage::PaneChrome(PaneChromeActionKind::CloseItem)
        },
    )?;
    let header = context.create_detached_component(document_id, HostStack::fill_row(6.0))?;
    context.append_child(header, pane_tabs)?;
    if state.pane_chrome_item_open && !state.pane_chrome_split {
        context.append_child(header, pane_split)?;
    }
    if state.pane_chrome_item_open {
        context.append_child(header, pane_close)?;
    }
    let pane = context.create_detached_component(
        document_id,
        PaneChrome::new()
            .header(header.stable_id())
            .tabs(pane_tabs.stable_id())
            .body(pane_tree.stable_id())
            .actions(pane_actions(
                state,
                pane_split.stable_id(),
                pane_close.stable_id(),
            )),
    )?;
    context.append_child(pane, header)?;
    context.append_child(pane, pane_tree)?;

    let empty = context.create_detached_component(
        document_id,
        EmptyState::new("没有选中的表面").message("选择一张卡片查看详情"),
    )?;
    let labeled = context.create_detached_component(
        document_id,
        LabeledValue::new("当前卡片", format!("{}", state.selected_surface_card)),
    )?;

    let root = context.create_detached_component(document_id, HostStack::canvas())?;
    let heading = context.create_detached_component(
        document_id,
        styled_text("表面层级", SemanticColorRole::Text, 14.0, 400),
    )?;
    let hint = context.create_detached_component(
        document_id,
        styled_text("基础、抬升与选中状态", SemanticColorRole::Muted, 11.0, 400),
    )?;
    let tab_row = panel(context, document_id, 8.0, None, &[], 0.0)?;
    let tab_label = context.create_detached_component(
        document_id,
        hugging_text("表面状态", SemanticColorRole::Text, 12.0, 400),
    )?;
    let tab_bar = context.create_detached_component(
        document_id,
        HostStack::fill_row(8.0).align(nana_ui::runtime::AlignSpec::Center),
    )?;
    context.append_child(tab_bar, tab_label)?;
    let tab_spacer = context.create_detached_component(document_id, HostStack::spacer())?;
    context.append_child(tab_bar, tab_spacer)?;
    context.append_child(tab_bar, tabs)?;
    context.append_child(tab_row, tab_bar)?;
    context.append_child(root, heading)?;
    context.append_child(root, hint)?;
    context.append_child(root, tab_row)?;
    context.append_child(root, surface_row)?;
    let tree_heading = context.create_detached_component(
        document_id,
        styled_text("层级树", SemanticColorRole::Text, 14.0, 400),
    )?;
    let tree_hint = context.create_detached_component(
        document_id,
        styled_text(
            "稳定节点 ID 驱动展开与选择",
            SemanticColorRole::Muted,
            11.0,
            400,
        ),
    )?;
    let tree_panel = panel(context, document_id, 8.0, None, &[], 0.0)?;
    context.append_child(tree_panel, tree)?;
    context.append_child(root, tree_heading)?;
    context.append_child(root, tree_hint)?;
    context.append_child(root, tree_panel)?;
    let pane_heading = context.create_detached_component(
        document_id,
        styled_text("Pane 组合", SemanticColorRole::Text, 14.0, 400),
    )?;
    let pane_hint = context.create_detached_component(
        document_id,
        styled_text(
            "动作只在具备真实 handler 时出现",
            SemanticColorRole::Muted,
            11.0,
            400,
        ),
    )?;
    let pane_panel = panel(
        context,
        document_id,
        0.0,
        Some(LengthSpec::Px(140.0)),
        &[],
        0.0,
    )?;
    context.append_child(pane_panel, pane)?;
    context.append_child(root, pane_heading)?;
    context.append_child(root, pane_hint)?;
    context.append_child(root, pane_panel)?;
    context.append_child(root, empty)?;
    context.append_child(root, labeled)?;

    Ok(SurfacesTree {
        root,
        tabs,
        surface_row,
        overview: [
            overview[0].expect("overview"),
            overview[1].expect("overview"),
            overview[2].expect("overview"),
        ],
        cards: [
            cards[0].expect("card"),
            cards[1].expect("card"),
            cards[2].expect("card"),
        ],
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
    let progress =
        context.create_detached_component(document_id, Progress::new(progress_value, 100.0))?;
    let spinner = context.create_detached_component(
        document_id,
        Spinner::new(if state.loading {
            "处理中"
        } else {
            "已完成"
        }),
    )?;
    let skeleton = context.create_detached_component(document_id, Skeleton::fill_width(8.0))?;
    let meter = context.create_detached_component(
        document_id,
        LevelMeter::new(f32::from(state.slider) / 100.0),
    )?;
    let badge = context.create_detached_component(
        document_id,
        StatusBadge::new(action_status(state), StatusTone::Info),
    )?;
    let validation = context.create_detached_component(
        document_id,
        ValidationMessage::new("等待操作", ValidationIntent::Warning),
    )?;
    let toast = context.create_detached_component(
        document_id,
        Toast::new(action_status(state), ToastTone::Info),
    )?;
    let dialog = context.create_detached_component(
        document_id,
        fill_action_button(
            if state.overlay.contains(&super::GalleryOverlay::Dialog) {
                "关闭对话框"
            } else {
                "打开对话框"
            },
            ButtonKind::Primary,
        ),
    )?;
    bind_event(context, dialog, Arc::clone(pending), |event: &Activate| {
        let _ = event;
        GalleryMessage::ToggleDialog
    })?;
    let context_btn = context.create_detached_component(
        document_id,
        fill_action_button("打开更多操作", ButtonKind::Subtle),
    )?;
    bind_event(
        context,
        context_btn,
        Arc::clone(pending),
        |event: &Activate| {
            let _ = event;
            GalleryMessage::ToggleContextMenu
        },
    )?;
    let image = context.create_detached_component(
        document_id,
        fill_action_button("查看图片", ButtonKind::Subtle),
    )?;
    bind_event(context, image, Arc::clone(pending), |event: &Activate| {
        let _ = event;
        GalleryMessage::ToggleImageViewer
    })?;
    let popover = context.create_detached_component(
        document_id,
        Popover::new()
            .trigger("查看当前状态")
            .open(state.popover_open),
    )?;
    bind_event(
        context,
        popover,
        Arc::clone(pending),
        |event: &PopoverToggled| {
            if event.open {
                GalleryMessage::TogglePopover
            } else {
                GalleryMessage::ClosePopover
            }
        },
    )?;
    bind_event(
        context,
        popover,
        Arc::clone(pending),
        |event: &PopoverClosed| {
            let _ = event;
            GalleryMessage::ClosePopover
        },
    )?;
    let popover_action = context
        .create_detached_component(document_id, popover_action_button(state.popover_open))?;
    bind_event(
        context,
        popover_action,
        Arc::clone(pending),
        |event: &Activate| {
            let _ = event;
            GalleryMessage::PrimaryAction
        },
    )?;
    context.append_child(popover, popover_action)?;

    let calendar = context
        .create_detached_component(document_id, CalendarHeatmap::new(gallery_calendar_data()))?;
    let calendar_status = context.create_detached_component(
        document_id,
        styled_text(
            state
                .calendar_active
                .as_ref()
                .map_or("移动指针查看日期".to_owned(), |cell| {
                    cell.title.clone()
                }),
            SemanticColorRole::Muted,
            10.0,
            400,
        ),
    )?;
    let action = context.create_detached_component(
        document_id,
        styled_text(action_status(state), SemanticColorRole::Muted, 10.0, 400),
    )?;

    let root = context.create_detached_component(document_id, HostStack::canvas())?;
    let heading = context.create_detached_component(
        document_id,
        styled_text("反馈", SemanticColorRole::Text, 14.0, 400),
    )?;
    context.append_child(root, heading)?;
    let row = context.create_detached_component(
        document_id,
        HostStack::fill_row(10.0).align(nana_ui::runtime::AlignSpec::Start),
    )?;
    let progress_panel = panel(
        context,
        document_id,
        8.0,
        Some(LengthSpec::Px(160.0)),
        &[],
        1.0,
    )?;
    let progress_label = context.create_detached_component(
        document_id,
        styled_text(
            if state.loading {
                "处理中"
            } else {
                "已完成"
            },
            SemanticColorRole::Text,
            13.0,
            400,
        ),
    )?;
    context.append_child(progress_panel, progress_label)?;
    context.append_child(progress_panel, progress)?;
    context.append_child(progress_panel, spinner)?;
    context.append_child(progress_panel, skeleton)?;
    context.append_child(progress_panel, meter)?;
    append_flex_child(context, document_id, row, progress_panel)?;
    let actions = context.create_detached_component(
        document_id,
        HostStack::panel(8.0)
            .width(LengthSpec::Px(140.0))
            .max_width(LengthSpec::Px(140.0))
            .grow(0.0)
            .shrink(0.0),
    )?;
    context.append_child(actions, dialog)?;
    context.append_child(actions, context_btn)?;
    context.append_child(actions, image)?;
    context.append_child(row, actions)?;
    context.append_child(root, row)?;
    let popover_row = context.create_detached_component(
        document_id,
        HostStack::column(0.0)
            .width(LengthSpec::Fill)
            .padding_xy(0.0, 8.0)
            .min_height(LengthSpec::Px(32.0))
            .grow(0.0)
            .shrink(0.0),
    )?;
    context.append_child(popover_row, popover)?;
    context.append_child(root, popover_row)?;
    let calendar_panel = panel(context, document_id, 6.0, None, &[], 0.0)?;
    let calendar_title = context.create_detached_component(
        document_id,
        styled_text("日历热力图", SemanticColorRole::Muted, 12.0, 400),
    )?;
    context.append_child(calendar_panel, calendar_title)?;
    context.append_child(calendar_panel, calendar)?;
    context.append_child(calendar_panel, calendar_status)?;
    context.append_child(root, calendar_panel)?;
    context.append_child(root, badge)?;
    context.append_child(root, validation)?;
    context.append_child(root, toast)?;
    context.append_child(root, action)?;

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
        context: context_btn,
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
    _pending: &Arc<Mutex<Vec<GalleryMessage>>>,
) -> Result<RichTextMount, FrameworkError> {
    let root = context.create_detached_component(document_id, HostStack::canvas())?;
    let heading = context.create_detached_component(
        document_id,
        styled_text("原生富文本", SemanticColorRole::Text, 20.0, 600),
    )?;
    let hint = context.create_detached_component(
        document_id,
        styled_text(
            "CommonMark、数学公式与图表共享同一 Runtime Scene 渲染路径。",
            SemanticColorRole::Muted,
            12.0,
            400,
        ),
    )?;
    let markdown = context.create_detached_component(document_id, state.markdown.clone())?;
    context.assemble_markdown(markdown)?;
    let link_status = context.create_detached_component(
        document_id,
        styled_text(
            state
                .opened_markdown_link
                .as_ref()
                .map_or(String::new(), |link| format!("已选择链接：{link}")),
            SemanticColorRole::Accent,
            11.0,
            400,
        ),
    )?;
    context.append_child(root, heading)?;
    context.append_child(root, hint)?;
    context.append_child(root, markdown)?;
    context.append_child(root, link_status)?;
    Ok((root, markdown, link_status))
}

fn mount_graph(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    state: &GalleryState,
    pending: &Arc<Mutex<Vec<GalleryMessage>>>,
) -> Result<GraphMount, FrameworkError> {
    let root = context.create_detached_component(document_id, HostStack::canvas())?;
    let toolbar = context.create_detached_component(
        document_id,
        HostStack::fill_row(10.0).align(nana_ui::runtime::AlignSpec::Center),
    )?;
    let title = context.create_detached_component(
        document_id,
        hugging_text("节点图", SemanticColorRole::Text, 14.0, 400),
    )?;
    let selection = context.create_detached_component(
        document_id,
        hugging_text(
            graph_selection_label(state),
            SemanticColorRole::Muted,
            11.0,
            400,
        ),
    )?;
    let reset = context.create_detached_component(
        document_id,
        Button::new("重置视图")
            .kind(ButtonKind::Text)
            .size(ControlSize::Small),
    )?;
    bind_event(context, reset, Arc::clone(pending), |event: &Activate| {
        let _ = event;
        GalleryMessage::ResetGraphViewport
    })?;
    context.append_child(toolbar, title)?;
    context.append_child(toolbar, selection)?;
    context.append_child(toolbar, reset)?;
    let graph = context.create_detached_component(
        document_id,
        GraphCanvas::new("gallery", state.graph.clone())
            .viewport(state.graph_viewport)
            .selection(state.graph_selection.clone()),
    )?;
    bind_event(
        context,
        graph,
        Arc::clone(pending),
        |event: &GraphCanvasEvent| GalleryMessage::Graph(event.clone()),
    )?;
    context.append_child(root, toolbar)?;
    context.append_child(root, graph)?;
    Ok((root, graph, selection, reset))
}

fn mount_workspace(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    state: &GalleryState,
    pending: &Arc<Mutex<Vec<GalleryMessage>>>,
) -> Result<WorkspaceTree, FrameworkError> {
    let mut panels = Vec::new();
    let mut contents = std::collections::HashMap::new();
    for (id, title, hint) in DOCK_PANELS {
        let panel = context
            .create_detached_component(document_id, HostStack::fill_column(5.0).padding(10.0))?;
        let heading = context.create_detached_component(
            document_id,
            styled_text(title, SemanticColorRole::Text, 12.0, 400),
        )?;
        let detail = context.create_detached_component(
            document_id,
            styled_text(hint, SemanticColorRole::Muted, 10.0, 400),
        )?;
        context.append_child(panel, heading)?;
        context.append_child(panel, detail)?;
        contents.insert(id.to_owned(), panel.stable_id());
        panels.push((id.to_owned(), panel));
    }
    let dock = context
        .create_detached_component(document_id, runtime_dock_from_workspace(state, &contents))?;
    for (_, panel) in &panels {
        context.append_child(dock, *panel)?;
    }
    context.assemble_dock(dock)?;

    let locked = state.dock_locked;
    let hidden_assets = !state.dock_is_visible("gallery.assets");
    let lock = context.create_detached_component(
        document_id,
        Button::new(if locked { "解锁 Dock" } else { "锁定 Dock" })
            .kind(ButtonKind::Subtle)
            .size(ControlSize::Small),
    )?;
    let hide = context.create_detached_component(
        document_id,
        Button::new(if hidden_assets {
            "恢复 Assets"
        } else {
            "隐藏 Assets"
        })
        .kind(ButtonKind::Subtle)
        .size(ControlSize::Small),
    )?;
    let reset = context.create_detached_component(
        document_id,
        Button::new("重置 Dock")
            .kind(ButtonKind::Subtle)
            .size(ControlSize::Small),
    )?;
    bind_event(context, reset, Arc::clone(pending), |event: &Activate| {
        let _ = event;
        GalleryMessage::Dock(GalleryDock::Reset)
    })?;
    let status = context
        .create_detached_component(document_id, workspace_status_text(dock_status(state)))?;

    let popup_title =
        context.create_detached_component(document_id, AppTitleBar::new("弹窗标题"))?;
    let popup_body = context.create_detached_component(
        document_id,
        HostStack::column(4.0).padding(12.0).grow(0.0).shrink(0.0),
    )?;
    let popup_heading = context.create_detached_component(
        document_id,
        styled_text("独立弹窗内容", SemanticColorRole::Text, 13.0, 400),
    )?;
    let popup_hint = context.create_detached_component(
        document_id,
        styled_text("快速创建并管理项目", SemanticColorRole::Muted, 11.0, 400),
    )?;
    context.append_child(popup_body, popup_heading)?;
    context.append_child(popup_body, popup_hint)?;
    let popup = context.create_detached_component(
        document_id,
        AppShell::new()
            .title_bar(popup_title.stable_id())
            .body(popup_body.stable_id()),
    )?;
    context.append_child(popup, popup_title)?;
    context.append_child(popup, popup_body)?;

    let root = context.create_detached_component(
        document_id,
        HostStack::fill_column(WORKSPACE_CANVAS_GAP)
            .padding(WORKSPACE_CANVAS_PADDING)
            .background(SemanticColorRole::Background)
            .grow(0.0),
    )?;
    let tools = context.create_detached_component(
        document_id,
        HostStack::fill_row(8.0)
            .align(nana_ui::runtime::AlignSpec::Center)
            .padding_xy(12.0, 8.0)
            .background(SemanticColorRole::Surface)
            .grow(0.0)
            .shrink(0.0),
    )?;
    context.append_child(tools, lock)?;
    context.append_child(tools, hide)?;
    context.append_child(tools, reset)?;
    context.append_child(tools, status)?;
    context.append_child(root, tools)?;
    let dock_frame = context.create_detached_component(
        document_id,
        HostStack::column(0.0)
            .height(LengthSpec::Px(WORKSPACE_DOCK_HEIGHT))
            .min_height(LengthSpec::Px(WORKSPACE_DOCK_HEIGHT))
            .grow(0.0)
            .shrink(0.0),
    )?;
    context.append_child(dock_frame, dock)?;
    context.append_child(root, dock_frame)?;
    let popup_frame = context.create_detached_component(
        document_id,
        HostStack::column(0.0)
            .width(LengthSpec::Px(WORKSPACE_POPUP_WIDTH))
            .height(LengthSpec::Px(WORKSPACE_POPUP_HEIGHT))
            .min_height(LengthSpec::Px(WORKSPACE_POPUP_HEIGHT))
            .background(SemanticColorRole::Surface)
            .grow(0.0)
            .shrink(0.0),
    )?;
    context.append_child(popup_frame, popup)?;
    context.append_child(root, popup_frame)?;

    Ok(WorkspaceTree {
        root,
        dock,
        panels,
        lock,
        hide,
        _reset: reset,
        status,
        _popup: popup,
    })
}

fn mount_inspector(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    state: &GalleryState,
    pending: &Arc<Mutex<Vec<GalleryMessage>>>,
) -> Result<InspectorTree, FrameworkError> {
    let collapse = context.create_detached_component(
        document_id,
        Button::new("收起")
            .kind(ButtonKind::Text)
            .size(ControlSize::Small),
    )?;
    bind_event(
        context,
        collapse,
        Arc::clone(pending),
        |event: &Activate| {
            let _ = event;
            GalleryMessage::Workspace(WorkspaceAction::ToggleRegion(RegionId::Inspector))
        },
    )?;
    let radius = state.appearance.standard_radius().round() as u8;
    let slider = context.create_detached_component(
        document_id,
        fill_range_field(
            nana_ui::runtime::RangeField::new(f64::from(radius), 0.0, 24.0, 1.0)
                .expect("radius range")
                .label("标准圆角")
                .unit("px"),
        ),
    )?;
    bind_event(
        context,
        slider,
        Arc::clone(pending),
        |event: &RangeChanged| GalleryMessage::SetStandardRadius(event.value.round() as u8),
    )?;
    let corners = context.create_detached_component(
        document_id,
        Switch::new("主区域圆角", state.appearance.workspace_corners_enabled()),
    )?;
    bind_event(
        context,
        corners,
        Arc::clone(pending),
        |event: &ToggleChanged| GalleryMessage::SetWorkspaceCorners(event.checked),
    )?;
    let slot = context.create_detached_component(document_id, HostStack::region_slot())?;
    let root = context.create_detached_component(
        document_id,
        HostStack::fill_column(10.0)
            .padding_xy(12.0, 10.0)
            .grow(0.0),
    )?;
    let heading = context.create_detached_component(
        document_id,
        HostStack::fill_row(8.0)
            .align(nana_ui::runtime::AlignSpec::Center)
            .grow(0.0),
    )?;
    let title = context.create_detached_component(
        document_id,
        hugging_text("检查器", SemanticColorRole::Muted, 12.0, 700),
    )?;
    context.append_child(heading, title)?;
    let heading_spacer = context.create_detached_component(document_id, HostStack::spacer())?;
    context.append_child(heading, heading_spacer)?;
    context.append_child(heading, collapse)?;
    context.append_child(root, heading)?;
    context.append_child(root, slider)?;
    context.append_child(root, corners)?;
    context.append_child(slot, root)?;
    Ok(InspectorTree {
        slot,
        _root: root,
        _collapse: collapse,
        radius: slider,
        corners,
    })
}

fn mount_bottom(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    pending: &Arc<Mutex<Vec<GalleryMessage>>>,
) -> Result<(Entity<HostStack>, Entity<Button>), FrameworkError> {
    let collapse = context.create_detached_component(
        document_id,
        Button::new("收起")
            .kind(ButtonKind::Text)
            .size(ControlSize::Small),
    )?;
    bind_event(
        context,
        collapse,
        Arc::clone(pending),
        |event: &Activate| {
            let _ = event;
            GalleryMessage::Workspace(WorkspaceAction::ToggleRegion(RegionId::Diagnostics))
        },
    )?;
    let slot = context.create_detached_component(document_id, HostStack::region_slot())?;
    let root = context.create_detached_component(
        document_id,
        HostStack::fill_column(8.0).padding_xy(12.0, 8.0).grow(0.0),
    )?;
    let heading = context.create_detached_component(
        document_id,
        HostStack::fill_row(8.0)
            .align(nana_ui::runtime::AlignSpec::Center)
            .grow(0.0),
    )?;
    let title = context.create_detached_component(
        document_id,
        hugging_text("底部面板", SemanticColorRole::Muted, 12.0, 700),
    )?;
    context.append_child(heading, title)?;
    let heading_spacer = context.create_detached_component(document_id, HostStack::spacer())?;
    context.append_child(heading, heading_spacer)?;
    context.append_child(heading, collapse)?;
    let status = context.create_detached_component(
        document_id,
        StatusBadge::new("布局就绪", StatusTone::Success),
    )?;
    context.append_child(root, heading)?;
    context.append_child(root, status)?;
    context.append_child(slot, root)?;
    Ok((slot, collapse))
}

fn mount_toolbar(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    pending: &Arc<Mutex<Vec<GalleryMessage>>>,
) -> Result<(Entity<HostStack>, Entity<Button>), FrameworkError> {
    let reset = context.create_detached_component(
        document_id,
        Button::new("恢复默认")
            .kind(ButtonKind::Text)
            .size(ControlSize::Small),
    )?;
    bind_event(context, reset, Arc::clone(pending), |event: &Activate| {
        let _ = event;
        GalleryMessage::ResetWorkspaceLayout
    })?;
    let slot = context.create_detached_component(document_id, HostStack::region_slot())?;
    let root = context.create_detached_component(
        document_id,
        HostStack::fill_row(8.0)
            .align(nana_ui::runtime::AlignSpec::Center)
            .height(LengthSpec::Fill)
            .padding_xy(10.0, 0.0)
            .grow(0.0),
    )?;
    let title = context.create_detached_component(
        document_id,
        hugging_text("工作区", SemanticColorRole::Text, 13.0, 700),
    )?;
    context.append_child(root, title)?;
    let spacer = context.create_detached_component(document_id, HostStack::spacer())?;
    context.append_child(root, spacer)?;
    context.append_child(root, reset)?;
    context.append_child(slot, root)?;
    Ok((slot, reset))
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
        let _ = context.update_component(dropdown, |field, _| {
            *field = gallery_dropdown(state, placeholder, field.size);
        });
    }
    let _ = context.update_component(tree.field_status, |label, _| {
        *label = field_status_text(state);
    });
    let _ = context.update_component(tree.checkbox, |box_, _| {
        *box_ = Checkbox::new("启用选项", state.checked);
    });
    let _ = context.update_component(tree.switch, |switch, _| {
        *switch = Switch::new("允许编辑说明", state.switched).disabled(!state.checked);
    });
    let _ = context.update_component(tree.range, |range, _| {
        range.value = f64::from(state.slider);
    });
    let _ = context.update_component(tree.search, |search, _| {
        *search = gallery_search(state);
    });
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
            11.0,
            400,
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
            11.0,
            400,
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
        *label = styled_text(action_status(state), SemanticColorRole::Muted, 10.0, 400);
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

fn append_flex_child<C: View>(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    parent: Entity<HostStack>,
    child: Entity<C>,
) -> Result<(), FrameworkError> {
    let cell = context.create_detached_component(document_id, HostStack::flex_child())?;
    context.append_child(cell, child)?;
    context.append_child(parent, cell)?;
    Ok(())
}

fn filling_panel(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    gap: f32,
) -> Result<Entity<HostStack>, FrameworkError> {
    context.create_detached_component(
        document_id,
        HostStack::panel(gap)
            .height(LengthSpec::Fill)
            .min_height(LengthSpec::Px(0.0))
            .grow(1.0),
    )
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
    let mut text = hugging_text(value, SemanticColorRole::Muted, 12.0, 400);
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
        11.0,
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
    let mut item = ListItem::new(label)
        .size(size)
        .selected(selected)
        .disabled(disabled)
        .slots(list_item_slots(leading, content, trailing));
    let layout = std::sync::Arc::make_mut(&mut item.style.layout);
    layout.width = Some(LengthSpec::Fill);
    layout.min_width = Some(LengthSpec::Px(0.0));
    layout.flex_grow = Some(0.0);
    layout.flex_shrink = Some(0.0);
    layout.white_space_nowrap = true;
    layout.text_overflow_ellipsis = true;
    item
}

fn panel(
    context: &mut nana_ui::runtime::AppContext,
    document_id: DocumentId,
    gap: f32,
    height: Option<LengthSpec>,
    leading: &[StableNodeId],
    grow: f32,
) -> Result<Entity<HostStack>, FrameworkError> {
    let mut stack = HostStack::panel(gap)
        .grow(grow)
        .min_width(LengthSpec::Px(0.0));
    if let Some(height) = height {
        stack = stack.height(height);
    }
    let panel = context.create_detached_component(document_id, stack)?;
    for child in leading {
        let mut mutations = nana_ui::runtime::MutationQueue::new();
        mutations.insert(panel.stable_id(), *child, None);
        context.commit_mutations(mutations)?;
    }
    Ok(panel)
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
    styled_text(copy, color, 12.0, 400)
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
    if !state.pane_chrome_item_open {
        PaneTreeNode::leaf_content("empty", empty.stable_id())
    } else if state.pane_chrome_split {
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
    let mut actions = Vec::new();
    if state.pane_chrome_item_open && !state.pane_chrome_split {
        actions.push(
            PaneChromeAction::new(PaneChromeActionKind::SplitHorizontal, "左右分栏").target(split),
        );
    }
    if state.pane_chrome_item_open {
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
            GalleryMessage::SetDropdown(nana_ui::DropdownEvent::Select(parse(value)))
        }
        DropdownEvent::Toggle(value) => {
            GalleryMessage::SetDropdown(nana_ui::DropdownEvent::Toggle(parse(value)))
        }
        DropdownEvent::Opened => GalleryMessage::SetDropdown(nana_ui::DropdownEvent::Opened),
        DropdownEvent::Closed => GalleryMessage::SetDropdown(nana_ui::DropdownEvent::Closed),
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
