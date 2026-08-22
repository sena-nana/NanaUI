use std::cell::OnceCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nana_ui::LogicalPoint;
use nana_ui::command::{
    ActionDescriptor, ActionId, ActionPickerNavigation, ActionPickerState, ActionRegistry,
    ContextPredicate, KeyBinding, KeyContext, KeyModifiers, KeyStroke, Keymap, KeymapMatch,
    KeymapState,
};
use nana_ui::components::{
    CalendarHeatmapActiveCell, CalendarHeatmapDatum, CalendarHeatmapEvent, CalendarHeatmapModel,
    CalendarHeatmapOptions, CommandPaletteEvent, CommandPaletteItem, ContextMenuItem,
    DropdownEvent, NativeMarkdown, SearchDropdownOption, TreeViewEvent, XYPadEvent, XYPadValue,
    build_calendar_heatmap_model,
};
use nana_ui::dialog::{DialogClosePolicy, DialogCloseTrigger};
use nana_ui::icons::Icon;
use nana_ui::layout::{
    RegionId, RegionPlacement, RegionRole, RegionScope, RegionState, WorkspaceLayout,
};
use nana_ui::menu::{MenuConfirmation, MenuSelection};
use nana_ui::overlay::ExclusiveOverlay;
use nana_ui::runtime::{FrameworkError, RuntimeDocument, UiScene};
use nana_ui::selection::{SelectionMove, SingleSelection};
use nana_ui::settings::{
    AppearanceSettings, BackdropTarget, SettingsModel, SettingsState, SettingsTab, SettingsTabId,
    WindowMaterialMode,
};
use nana_ui::theme::{ThemeMode, ThemeModeExt, ThemeTokens};
use nana_ui::window_chrome::{WindowChromeEvent, WindowChromeState};
use nana_ui::workspace::{WorkspaceAction, WorkspaceController};
use nana_ui::{
    AppearanceEvent, DockWorkspace, DockWorkspaceEvent, GraphCanvasEvent, GraphEdge, GraphEndpoint,
    GraphModel, GraphNode, GraphPoint, GraphPort, GraphPortKind, GraphPortSide, GraphSelection,
    GraphSize, GraphViewport, MaterialOutcome, PaneChromeActionKind, RuntimeProgram,
    RuntimeProgramContext, RuntimeProgramUpdate, RuntimeRedraw, SplitAxis, SplitPaneAction,
    SplitPaneController,
};
use nana_ui_platform::{
    InputEvent, WindowCommand, WindowEvent, WindowId, WindowRole, WindowSettings,
};

#[path = "views/graph.rs"]
mod graph_view;
#[path = "views/root.rs"]
mod root_view;
mod runtime_gallery;
mod runtime_host;
mod runtime_overlays;
mod runtime_settings;
#[path = "views/settings.rs"]
mod settings_view;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GallerySection {
    Controls,
    Surfaces,
    Feedback,
    RichText,
    Graph,
    Workspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceView {
    Overview,
    Cards,
}

impl SurfaceView {
    const fn index(self) -> usize {
        match self {
            Self::Overview => 0,
            Self::Cards => 1,
        }
    }

    const fn from_index(index: usize) -> Self {
        if index == 1 {
            Self::Cards
        } else {
            Self::Overview
        }
    }
}

/// Gallery dock operations mapped onto [`DockWorkspace`] / [`DockWorkspaceEvent`].
#[derive(Debug, Clone, PartialEq)]
pub enum GalleryDock {
    ActivateTab(Arc<str>),
    Hide(Arc<str>),
    Show(Arc<str>),
    Float {
        id: Arc<str>,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    Focus(Arc<str>),
    CloseFloating(Arc<str>),
    MoveFloating {
        id: Arc<str>,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },
    SetLocked(bool),
    Reset,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GalleryMessage {
    Workspace(WorkspaceAction),
    SplitPane(SplitPaneAction),
    Dock(GalleryDock),
    ToggleTheme,
    SetTheme(ThemeMode),
    SetStandardRadius(u8),
    SetWorkspaceCorners(bool),
    SetWindowMaterial(WindowMaterialMode),
    SetBackdropTarget(BackdropTarget),
    SetBackdropOpacity(f32),
    SetTitlebarFollowsSidebar(bool),
    ResetAppearance,
    SelectSection(GallerySection),
    Graph(GraphCanvasEvent),
    ResetGraphViewport,
    OpenMarkdownLink(String),
    OpenSettings,
    BackFromSettings,
    SelectSettingsTab(SettingsTabId),
    ToggleWorkspaceSettingsDetails,
    ResetWorkspaceLayout,
    PrimaryAction,
    ToggleLoading,
    LoadingTick,
    InputChanged(String),
    ToggleCheck(bool),
    ToggleSwitch(bool),
    SetSlider(u8),
    SetXYPad(XYPadEvent),
    SetDropdown(DropdownEvent<u8>),
    SelectSearchResult(u8),
    SearchDropdownInput(String),
    CalendarHeatmap(CalendarHeatmapEvent),
    SelectListItem(usize),
    SelectSurfaceCard(usize),
    SelectSurfaceView(SurfaceView),
    NavigateSurfaceView(SelectionMove),
    TreeView(TreeViewEvent<String>),
    PaneChrome(PaneChromeActionKind),
    ToggleDialog,
    ConfirmDialog,
    RequestDialogClose(DialogCloseTrigger),
    DismissOverlay,
    OverlayInteraction,
    ToggleContextMenu,
    OpenContextMenu {
        x: f32,
        y: f32,
    },
    ToggleImageViewer,
    RequestImageViewerClose(DialogCloseTrigger),
    TogglePopover,
    ClosePopover,
    ContextMenu(GalleryContextMenuEvent),
    ToggleCommandPalette,
    CommandPalette(CommandPaletteEvent),
    NavigateCommandPalette(ActionPickerNavigation),
    KeyStroke(KeyStroke),
    WindowChrome(WindowChromeEvent),
    /// Host-applied [`MaterialOutcome`] from `nana-window`.
    MaterialApplied(MaterialOutcome),
    SettingsRuntime(runtime_settings::SettingsRuntimeInput),
    GalleryRuntime(runtime_host::RuntimeSceneInput),
    OverlayRuntime(runtime_host::RuntimeSceneInput),
    SetEditorText(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GalleryContextMenuEvent {
    Search(String),
    Select(String),
    Dismiss,
    OpenSubmenu(Vec<usize>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextAction {
    Duplicate,
    Rename,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GalleryOverlay {
    CommandPalette,
    ContextMenu,
    Dialog,
    ImageViewer,
}

#[derive(Debug)]
pub struct GalleryState {
    theme: ThemeMode,
    appearance: AppearanceSettings,
    workspace: WorkspaceController,
    settings_workspace: WorkspaceController,
    split_pane: SplitPaneController,
    dock: DockWorkspace,
    dock_locked: bool,
    dock_events: Vec<DockWorkspaceEvent>,
    dock_window_commands: Vec<WindowCommand>,
    settings_model: SettingsModel,
    settings: SettingsState,
    section: GallerySection,
    settings_open: bool,
    workspace_settings_expanded: bool,
    input: String,
    checked: bool,
    switched: bool,
    loading: bool,
    loading_ticks: u8,
    slider: u8,
    xy_pad: XYPadValue,
    dropdown_values: Vec<u8>,
    search_dropdown_options: Vec<SearchDropdownOption>,
    search_dropdown_query: String,
    search_selection: Option<u8>,
    #[allow(dead_code)]
    calendar_model: OnceCell<CalendarHeatmapModel>,
    calendar_active: Option<CalendarHeatmapActiveCell>,
    selected_item: usize,
    selected_surface_card: usize,
    surface_selection: SingleSelection,
    tree_expanded: bool,
    tree_selected: String,
    pane_chrome_split: bool,
    pane_chrome_item_open: bool,
    overlay: ExclusiveOverlay<GalleryOverlay>,
    dialog_policy: DialogClosePolicy,
    menu_confirmation: MenuConfirmation<ContextAction>,
    context_action: Option<ContextAction>,
    context_anchor: Option<(f32, f32)>,
    context_items: OnceCell<Vec<ContextMenuItem>>,
    context_query: String,
    context_path: Vec<usize>,
    action_registry: ActionRegistry,
    keymap: Keymap,
    keymap_state: KeymapState,
    action_picker: ActionPickerState,
    palette_action: Option<ActionId>,
    popover_open: bool,
    confirmed_actions: u32,
    markdown: NativeMarkdown,
    opened_markdown_link: Option<String>,
    graph: GraphModel,
    graph_viewport: GraphViewport,
    graph_selection: Option<GraphSelection>,
    editor: String,
    primary_clicks: u32,
    window_chrome: WindowChromeState,
    /// Latest material application outcome from the Scene host path.
    material_outcome: MaterialOutcome,
    window_size: Option<(f32, f32)>,
    settings_runtime: Option<runtime_settings::GallerySettingsRuntime>,
    gallery_runtime: Option<runtime_gallery::GalleryRuntime>,
    overlay_runtime: Option<runtime_overlays::GalleryOverlaysRuntime>,
}

impl Default for GalleryState {
    fn default() -> Self {
        Self::new()
    }
}

impl GalleryState {
    pub fn new() -> Self {
        let settings_model = settings_model();
        let settings = SettingsState::new(&settings_model);
        let mut state = Self {
            theme: ThemeMode::Dark,
            appearance: AppearanceSettings::default(),
            workspace: WorkspaceController::with_layout(gallery_layout(false)),
            settings_workspace: WorkspaceController::with_layout(settings_layout()),
            split_pane: SplitPaneController::new(SplitAxis::Vertical, 120.0, 64.0, 280.0),
            dock: gallery_dock_workspace(),
            dock_locked: false,
            dock_events: Vec::new(),
            dock_window_commands: Vec::new(),
            settings_model,
            settings,
            section: GallerySection::Controls,
            settings_open: false,
            workspace_settings_expanded: true,
            input: String::new(),
            checked: true,
            switched: true,
            loading: false,
            loading_ticks: 0,
            slider: 58,
            xy_pad: XYPadValue::new(0.65, 0.35),
            dropdown_values: vec![50],
            search_dropdown_options: vec![
                SearchDropdownOption::new("1", "第一个选项").hint("Alpha"),
                SearchDropdownOption::new("2", "第二个选项").hint("Beta"),
                SearchDropdownOption::new("3", "第三个选项").hint("Gamma"),
            ],
            search_dropdown_query: String::new(),
            search_selection: Some(2),
            calendar_model: OnceCell::new(),
            calendar_active: None,
            selected_item: 0,
            selected_surface_card: 0,
            surface_selection: SingleSelection::new(0),
            tree_expanded: true,
            tree_selected: "src/lib.rs".to_owned(),
            pane_chrome_split: false,
            pane_chrome_item_open: true,
            overlay: ExclusiveOverlay::new(),
            dialog_policy: DialogClosePolicy::default(),
            menu_confirmation: MenuConfirmation::new(),
            context_action: None,
            context_anchor: None,
            context_items: OnceCell::new(),
            context_query: String::new(),
            context_path: Vec::new(),
            action_registry: gallery_action_registry(),
            keymap: gallery_keymap(),
            keymap_state: KeymapState::default(),
            action_picker: ActionPickerState::new(),
            palette_action: None,
            popover_open: false,
            confirmed_actions: 0,
            markdown: NativeMarkdown::parse(MARKDOWN_FIXTURE),
            opened_markdown_link: None,
            graph: graph_view::gallery_graph(),
            graph_viewport: GraphViewport::new(GraphPoint::new(72.0, 96.0), 1.0),
            graph_selection: None,
            editor: "示例说明\n用于展示多行文本编辑".to_owned(),
            primary_clicks: 0,
            window_chrome: WindowChromeState::default(),
            material_outcome: MaterialOutcome::chosen_solid(),
            window_size: None,
            settings_runtime: None,
            gallery_runtime: None,
            overlay_runtime: None,
        };
        state.refresh_gallery_runtime();
        state
    }

    pub fn theme_mode(&self) -> ThemeMode {
        self.theme
    }

    /// Flush retained Runtime documents so snapshot tooling can paint `UiScene`.
    pub fn flush_snapshot_scene(&mut self) {
        let (width, height) = self.window_size.unwrap_or(runtime_host::DEFAULT_VIEWPORT);
        if self.window_size != Some((width, height)) {
            self.update(GalleryMessage::Workspace(WorkspaceAction::WindowResized {
                width,
                height,
            }));
        } else if self.settings_open {
            self.refresh_settings_runtime();
        } else {
            self.refresh_gallery_runtime();
        }
        if self.overlay.is_open() {
            self.refresh_overlay_runtime();
        }
    }

    pub fn document(&self) -> Option<&RuntimeDocument> {
        self.runtime_document()
    }

    pub fn document_mut(&mut self) -> Option<&mut RuntimeDocument> {
        if self.settings_open {
            self.settings_runtime
                .as_mut()
                .map(runtime_settings::GallerySettingsRuntime::document_mut)
        } else {
            self.gallery_runtime
                .as_mut()
                .map(runtime_gallery::GalleryRuntime::document_mut)
        }
    }

    pub fn active_scene(&self) -> Option<&UiScene> {
        self.runtime_document().map(RuntimeDocument::scene)
    }

    /// Overlay lives in the primary Runtime document; same `UiScene` as
    /// [`Self::active_scene`]. Snapshot paint must not stack this on the base.
    pub fn overlay_scene(&self) -> Option<&UiScene> {
        self.overlay
            .is_open()
            .then(|| self.active_scene())
            .flatten()
    }

    /// Drive hover through Runtime input so snapshot paint includes pointer state.
    pub fn snapshot_hover(&mut self, x: f32, y: f32) {
        let point = LogicalPoint::new(x, y);
        if self.overlay.is_open() {
            self.update(GalleryMessage::OverlayRuntime(
                runtime_host::RuntimeSceneInput::PointerMove(point),
            ));
        } else if self.settings_open {
            self.update(GalleryMessage::SettingsRuntime(
                runtime_host::RuntimeSceneInput::PointerMove(point),
            ));
        } else {
            self.update(GalleryMessage::GalleryRuntime(
                runtime_host::RuntimeSceneInput::PointerMove(point),
            ));
        }
    }

    pub fn material_outcome(&self) -> MaterialOutcome {
        self.material_outcome
    }

    fn theme_tokens(&self) -> ThemeTokens {
        ThemeTokens::new(self.theme.colors(), self.appearance.metrics())
            .with_workspace_corners(self.appearance.workspace_corners_enabled())
            .with_backdrop(
                self.material_outcome.is_native(),
                self.appearance.backdrop_target(),
                self.appearance.backdrop_opacity(),
                self.appearance.titlebar_follows_sidebar(),
            )
    }

    /// Token alphas driven by Appearance backdrop target (test + diagnostics).
    ///
    /// Returns `(surface, background, titlebar)`.
    pub fn backdrop_region_alphas(&self) -> (f32, f32, f32) {
        let tokens = self.theme_tokens();
        (
            tokens.colors.surface.a,
            tokens.colors.background.a,
            tokens.titlebar.a,
        )
    }

    fn editor_enabled(&self) -> bool {
        self.checked && self.switched
    }

    #[allow(dead_code)]
    fn calendar_model(&self) -> &CalendarHeatmapModel {
        self.calendar_model.get_or_init(gallery_calendar_model)
    }

    fn context_items(&self) -> &[ContextMenuItem] {
        self.context_items
            .get_or_init(gallery_context_items)
            .as_slice()
    }

    pub fn update(&mut self, message: GalleryMessage) {
        let runtime_input = matches!(
            message,
            GalleryMessage::SettingsRuntime(_)
                | GalleryMessage::GalleryRuntime(_)
                | GalleryMessage::OverlayRuntime(_)
        );
        let overlay_input = matches!(message, GalleryMessage::OverlayRuntime(_));
        match message {
            GalleryMessage::WindowChrome(event) => {
                self.window_chrome.update(event);
            }
            GalleryMessage::MaterialApplied(outcome) => {
                self.material_outcome = outcome;
            }
            GalleryMessage::SettingsRuntime(input) => {
                self.handle_settings_runtime_input(input);
            }
            GalleryMessage::GalleryRuntime(input) => {
                self.handle_gallery_runtime_input(input);
            }
            GalleryMessage::OverlayRuntime(input) => {
                self.handle_overlay_runtime_input(input);
            }
            GalleryMessage::SetEditorText(value) => {
                self.editor = value;
            }
            GalleryMessage::Workspace(action) => {
                if let WorkspaceAction::WindowResized { width, height } = &action {
                    self.window_size = Some((*width, *height));
                }
                let synchronize_viewport = matches!(
                    &action,
                    WorkspaceAction::WindowResized { .. }
                        | WorkspaceAction::WindowScaleFactorChanged(_)
                );
                if self.settings_open {
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
            GalleryMessage::SplitPane(action) => {
                self.split_pane.update(action);
            }
            GalleryMessage::Dock(action) => self.apply_gallery_dock(action),
            GalleryMessage::ToggleTheme => self.theme = self.theme.toggle(),
            GalleryMessage::SetTheme(theme) => self.theme = theme,
            GalleryMessage::SetStandardRadius(radius) => {
                self.appearance.set_standard_radius(f32::from(radius));
            }
            GalleryMessage::SetWorkspaceCorners(enabled) => {
                self.appearance.set_workspace_corners_enabled(enabled);
            }
            GalleryMessage::SetWindowMaterial(mode) => {
                self.appearance.set_window_material(mode);
            }
            GalleryMessage::SetBackdropTarget(target) => {
                self.appearance.set_backdrop_target(target);
            }
            GalleryMessage::SetBackdropOpacity(opacity) => {
                self.appearance.set_backdrop_opacity(opacity);
            }
            GalleryMessage::SetTitlebarFollowsSidebar(enabled) => {
                self.appearance.set_titlebar_follows_sidebar(enabled);
            }
            GalleryMessage::ResetAppearance => {
                // Match Lilia `resetAppearanceDefaults` / AppearanceEvent::Reset:
                // appearance fields + ThemeMode::Light (not ThemeMode::default).
                self.appearance.reset();
                self.theme = AppearanceSettings::RESET_THEME;
            }
            GalleryMessage::SelectSection(section) => {
                self.section = section;
                self.settings_open = false;
                self.set_workspace_showcase_visible(section == GallerySection::Workspace);
                self.overlay.dismiss();
                self.menu_confirmation.clear();
                self.popover_open = false;
            }
            GalleryMessage::OpenMarkdownLink(link) => {
                self.opened_markdown_link = Some(link);
            }
            GalleryMessage::Graph(GraphCanvasEvent::SelectionChanged(selection)) => {
                self.graph_selection = selection;
            }
            GalleryMessage::Graph(
                GraphCanvasEvent::ViewportInput(viewport)
                | GraphCanvasEvent::ViewportChanged(viewport),
            ) => self.graph_viewport = viewport,
            GalleryMessage::Graph(
                GraphCanvasEvent::NodePositionInput { node, position }
                | GraphCanvasEvent::NodePositionChanged { node, position },
            ) => {
                let _ = self.graph.set_node_position(&node, position);
            }
            GalleryMessage::Graph(GraphCanvasEvent::ConnectionRequested { source, target }) => {
                let edge_id = format!("gallery-edge-{}", self.graph.edges().len() + 1);
                let _ = self.graph.add_edge(GraphEdge::new(edge_id, source, target));
            }
            GalleryMessage::ResetGraphViewport => self.reset_graph_viewport(),
            GalleryMessage::OpenSettings => {
                if let Some(size) = self
                    .workspace
                    .layout()
                    .region(&RegionId::Resources)
                    .and_then(RegionState::size_value)
                {
                    self.settings_workspace
                        .update(WorkspaceAction::SetRegionSize(RegionId::Resources, size));
                }
                self.settings_open = true;
                self.overlay.dismiss();
                self.menu_confirmation.clear();
                self.popover_open = false;
            }
            GalleryMessage::BackFromSettings => self.settings_open = false,
            GalleryMessage::SelectSettingsTab(tab) => {
                self.settings.select(&self.settings_model, &tab);
            }
            GalleryMessage::ToggleWorkspaceSettingsDetails => {
                self.workspace_settings_expanded = !self.workspace_settings_expanded;
            }
            GalleryMessage::ResetWorkspaceLayout => {
                self.workspace
                    .replace_layout(gallery_layout(self.section == GallerySection::Workspace));
            }
            GalleryMessage::PrimaryAction => {
                self.primary_clicks = self.primary_clicks.saturating_add(1);
            }
            GalleryMessage::ToggleLoading => {
                self.loading = true;
                self.loading_ticks = 0;
            }
            GalleryMessage::LoadingTick => {
                if self.loading {
                    self.loading_ticks = self.loading_ticks.saturating_add(1);
                    if self.loading_ticks >= 12 {
                        self.loading = false;
                        self.loading_ticks = 0;
                    }
                }
            }
            GalleryMessage::InputChanged(input) => self.input = input,
            GalleryMessage::ToggleCheck(value) => self.checked = value,
            GalleryMessage::ToggleSwitch(value) => self.switched = value,
            GalleryMessage::SetSlider(value) => self.slider = value.min(100),
            GalleryMessage::SetXYPad(event) => {
                self.xy_pad = match event {
                    XYPadEvent::Input(value) | XYPadEvent::Change(value) => value,
                };
            }
            GalleryMessage::SetDropdown(event) => match event {
                DropdownEvent::Select(value) => self.slider = value,
                DropdownEvent::Toggle(value) => {
                    if let Some(index) = self
                        .dropdown_values
                        .iter()
                        .position(|selected| *selected == value)
                    {
                        self.dropdown_values.remove(index);
                    } else {
                        self.dropdown_values.push(value);
                    }
                }
                DropdownEvent::Opened | DropdownEvent::Closed => {}
            },
            GalleryMessage::SelectSearchResult(value) => self.search_selection = Some(value),
            GalleryMessage::SearchDropdownInput(query) => self.search_dropdown_query = query,
            GalleryMessage::CalendarHeatmap(event) => match event {
                CalendarHeatmapEvent::CellEnter(cell) | CalendarHeatmapEvent::CellMove(cell) => {
                    self.calendar_active = Some(cell)
                }
                CalendarHeatmapEvent::CellLeave => self.calendar_active = None,
            },
            GalleryMessage::SelectListItem(index) => self.selected_item = index,
            GalleryMessage::SelectSurfaceCard(index) => self.selected_surface_card = index,
            GalleryMessage::SelectSurfaceView(view) => {
                self.surface_selection.select(view.index(), &[true, true]);
            }
            GalleryMessage::NavigateSurfaceView(movement) => {
                self.surface_selection.navigate(movement, &[true, true]);
            }
            GalleryMessage::TreeView(event) => match event {
                TreeViewEvent::Toggle(id) if id == "src" => {
                    self.tree_expanded = !self.tree_expanded;
                }
                TreeViewEvent::Toggle(_) => {}
                TreeViewEvent::Select(id) => self.tree_selected = id,
            },
            GalleryMessage::PaneChrome(action) => match action {
                PaneChromeActionKind::SplitHorizontal if self.pane_chrome_item_open => {
                    self.pane_chrome_split = true;
                }
                PaneChromeActionKind::CloseItem => {
                    self.pane_chrome_item_open = false;
                    self.pane_chrome_split = false;
                }
                _ => {}
            },
            GalleryMessage::ToggleDialog => {
                self.menu_confirmation.clear();
                self.overlay.toggle(GalleryOverlay::Dialog);
            }
            GalleryMessage::ConfirmDialog => {
                if self.overlay.contains(&GalleryOverlay::Dialog) {
                    self.confirmed_actions = self.confirmed_actions.saturating_add(1);
                    self.context_action = None;
                    self.overlay.dismiss();
                }
            }
            GalleryMessage::RequestDialogClose(trigger) => {
                if self.overlay.contains(&GalleryOverlay::Dialog) {
                    if self.dialog_policy.allows(trigger) {
                        self.overlay.dismiss();
                    }
                } else if trigger == DialogCloseTrigger::Escape {
                    self.overlay.dismiss();
                    self.menu_confirmation.clear();
                }
            }
            GalleryMessage::DismissOverlay => {
                self.action_picker.dismiss();
                self.overlay.dismiss();
                self.menu_confirmation.clear();
                self.context_anchor = None;
                self.context_query.clear();
                self.context_path.clear();
            }
            GalleryMessage::OverlayInteraction => {}
            GalleryMessage::ToggleContextMenu => {
                self.menu_confirmation.clear();
                self.context_query.clear();
                self.context_path.clear();
                self.context_anchor = None;
                self.overlay.toggle(GalleryOverlay::ContextMenu);
            }
            GalleryMessage::OpenContextMenu { x, y } => {
                self.menu_confirmation.clear();
                self.context_query.clear();
                self.context_path.clear();
                self.context_anchor = Some((x, y));
                self.overlay.open(GalleryOverlay::ContextMenu);
            }
            GalleryMessage::ToggleImageViewer => {
                self.menu_confirmation.clear();
                self.popover_open = false;
                self.overlay.toggle(GalleryOverlay::ImageViewer);
            }
            GalleryMessage::RequestImageViewerClose(trigger) => {
                if self.overlay.contains(&GalleryOverlay::ImageViewer)
                    && self.dialog_policy.allows(trigger)
                {
                    self.overlay.dismiss();
                }
            }
            GalleryMessage::TogglePopover => self.popover_open = !self.popover_open,
            GalleryMessage::ClosePopover => self.popover_open = false,
            GalleryMessage::ContextMenu(event) => match event {
                GalleryContextMenuEvent::Search(query) => {
                    self.context_query = query;
                    self.context_path.clear();
                }
                GalleryContextMenuEvent::OpenSubmenu(path) => self.context_path = path,
                GalleryContextMenuEvent::Select(value) => {
                    if !self.overlay.contains(&GalleryOverlay::ContextMenu) {
                        return;
                    }
                    let Some(action) = context_action_from_value(&value) else {
                        return;
                    };
                    let requires_confirmation = action == ContextAction::Remove;
                    if let MenuSelection::Confirmed(action) =
                        self.menu_confirmation.select(action, requires_confirmation)
                    {
                        self.apply_context_action(action);
                    }
                }
                GalleryContextMenuEvent::Dismiss => {
                    self.overlay.dismiss();
                    self.context_anchor = None;
                    self.menu_confirmation.clear();
                    self.context_query.clear();
                    self.context_path.clear();
                }
            },
            GalleryMessage::ToggleCommandPalette => self.toggle_command_palette(),
            GalleryMessage::CommandPalette(event) => self.update_command_palette(event),
            GalleryMessage::NavigateCommandPalette(navigation) => {
                self.navigate_command_palette(navigation);
            }
            GalleryMessage::KeyStroke(stroke) => {
                let context = self.action_context();
                if let KeymapMatch::Dispatch(action) = self.keymap.resolve(
                    &mut self.keymap_state,
                    stroke,
                    &context,
                    &self.action_registry,
                ) {
                    self.execute_gallery_action(action);
                }
            }
        }
        if self.settings_open && !runtime_input {
            self.refresh_settings_runtime();
        } else if !self.settings_open && !runtime_input {
            self.refresh_gallery_runtime();
        }
        if !overlay_input {
            self.refresh_overlay_runtime();
        }
    }

    fn apply_gallery_dock(&mut self, action: GalleryDock) {
        if self.dock_locked && !gallery_dock_allowed_when_locked(&action) {
            return;
        }
        match action {
            GalleryDock::ActivateTab(id) => {
                activate_runtime_dock_tab_in_workspace(&mut self.dock, id.as_ref());
            }
            GalleryDock::MoveFloating {
                id,
                x,
                y,
                width,
                height,
            } => {
                if self.dock.floating.iter().any(|item| item.id == id) {
                    self.apply_dock_workspace_event(DockWorkspaceEvent::MoveFloating {
                        id,
                        x,
                        y,
                        width,
                        height,
                    });
                }
            }
            GalleryDock::Hide(id) => {
                let _ = self.dock.hide(id.as_ref());
            }
            GalleryDock::Show(id) => {
                let _ = self.dock.show(id.as_ref());
            }
            GalleryDock::Float {
                id,
                x,
                y,
                width,
                height,
            } => {
                if id.as_ref() == DOCK_CENTER {
                    return;
                }
                let Some(event) = self.dock.float_item_at(id.as_ref(), x, y, width, height) else {
                    return;
                };
                self.record_dock_workspace_events([event]);
            }
            GalleryDock::Focus(id) => {
                activate_runtime_dock_tab_in_workspace(&mut self.dock, id.as_ref());
                if let Some(surface) = floating_surface_for_item(&self.dock, id.as_ref()) {
                    self.apply_dock_workspace_event(DockWorkspaceEvent::FocusFloating(surface));
                }
            }
            GalleryDock::CloseFloating(id) => {
                if self.dock.floating.iter().any(|item| item.id == id) {
                    self.apply_dock_workspace_event(DockWorkspaceEvent::CloseFloating(id));
                }
            }
            GalleryDock::SetLocked(locked) => {
                self.dock_locked = locked;
            }
            GalleryDock::Reset => {
                let closing = self
                    .dock
                    .floating
                    .iter()
                    .map(|surface| DockWorkspaceEvent::CloseFloating(Arc::clone(&surface.id)))
                    .collect::<Vec<_>>();
                self.dock = gallery_dock_workspace();
                self.dock_locked = false;
                self.record_dock_workspace_events(closing);
            }
        }
    }

    fn apply_dock_workspace_event(&mut self, event: DockWorkspaceEvent) {
        self.dock.apply(event.clone());
        self.record_dock_workspace_events([event]);
    }

    fn record_dock_workspace_events(
        &mut self,
        events: impl IntoIterator<Item = DockWorkspaceEvent>,
    ) {
        let events = events.into_iter().collect::<Vec<_>>();
        if events.is_empty() {
            return;
        }
        self.dock_window_commands
            .extend(runtime_dock_window_commands(events.iter().cloned()));
        self.dock_events.extend(events);
    }

    fn dock_is_visible(&self, id: &str) -> bool {
        self.dock.is_visible(id)
    }

    fn action_context(&self) -> KeyContext {
        let mut context = KeyContext::new(["workspace"]);
        if self.section == GallerySection::Graph && !self.settings_open {
            context.insert("graph");
        }
        context
    }

    fn palette_items(&self) -> Vec<CommandPaletteItem> {
        let context = self.action_context();
        self.action_registry
            .search(self.action_picker.query(), &context)
            .into_iter()
            .map(|matched| {
                let mut item = CommandPaletteItem::new(
                    matched.action.id.clone(),
                    matched.action.label.clone(),
                );
                if let Some(category) = &matched.action.category {
                    item = item.category(category.clone());
                }
                if let Some(shortcut) =
                    self.keymap
                        .binding_label(&matched.action.id, &context, &self.action_registry)
                {
                    item = item.shortcut(shortcut);
                }
                item
            })
            .collect()
    }

    fn toggle_command_palette(&mut self) {
        if self.overlay.contains(&GalleryOverlay::CommandPalette) {
            self.action_picker.dismiss();
            self.overlay.dismiss();
        } else {
            self.action_picker.open(None);
            self.overlay.open(GalleryOverlay::CommandPalette);
        }
    }

    fn update_command_palette(&mut self, event: CommandPaletteEvent) {
        if !self.overlay.contains(&GalleryOverlay::CommandPalette) {
            return;
        }
        match event {
            CommandPaletteEvent::Search(query) => {
                self.action_picker.set_query(query);
                let count = self.palette_items().len();
                self.action_picker.sync_results(count);
            }
            CommandPaletteEvent::Select(action) => self.execute_gallery_action(action),
            CommandPaletteEvent::Navigate(navigation) => {
                self.navigate_command_palette(navigation);
            }
            CommandPaletteEvent::Dismiss => {
                self.action_picker.dismiss();
                self.overlay.dismiss();
            }
        }
    }

    fn navigate_command_palette(&mut self, navigation: ActionPickerNavigation) {
        if !self.overlay.contains(&GalleryOverlay::CommandPalette) {
            if navigation == ActionPickerNavigation::Dismiss {
                self.update(GalleryMessage::RequestDialogClose(
                    DialogCloseTrigger::Escape,
                ));
            }
            return;
        }
        let items = self.palette_items();
        match navigation {
            ActionPickerNavigation::Confirm => {
                if let Some(item) = items.get(self.action_picker.selected()) {
                    self.execute_gallery_action(item.action.clone());
                }
            }
            ActionPickerNavigation::Dismiss => {
                self.action_picker.dismiss();
                self.overlay.dismiss();
            }
            navigation => self.action_picker.navigate(navigation, items.len()),
        }
    }

    fn execute_gallery_action(&mut self, action: ActionId) {
        self.palette_action = Some(action.clone());
        if action.as_str() == "workspace.command_palette" {
            self.toggle_command_palette();
            return;
        }
        if self.action_picker.is_open() {
            self.action_picker.confirm(action.clone());
            self.overlay.dismiss();
        }
        match action.as_str() {
            "appearance.toggle_theme" => self.theme = self.theme.toggle(),
            "workspace.toggle_sidebar" => {
                self.active_workspace_mut()
                    .update(WorkspaceAction::ToggleRegion(RegionId::Resources));
            }
            "workspace.reset_layout" => {
                self.workspace
                    .replace_layout(gallery_layout(self.section == GallerySection::Workspace));
            }
            "graph.reset_viewport" => self.reset_graph_viewport(),
            _ => {}
        }
    }

    fn active_workspace_mut(&mut self) -> &mut WorkspaceController {
        if self.settings_open {
            &mut self.settings_workspace
        } else {
            &mut self.workspace
        }
    }

    fn apply_context_action(&mut self, action: ContextAction) {
        self.context_action = Some(action);
        self.overlay.dismiss();
        self.menu_confirmation.clear();
        match action {
            ContextAction::Duplicate => {
                self.primary_clicks = self.primary_clicks.saturating_add(1);
            }
            ContextAction::Rename => {
                self.editor = "已重命名项目".to_owned();
            }
            ContextAction::Remove => {
                self.selected_item = 0;
            }
        }
    }
}

const LOADING_TICK: Duration = Duration::from_millis(100);

/// Scene-host application: one Runtime document per window.
pub struct GalleryApp {
    state: GalleryState,
    dock_windows: HashMap<WindowId, runtime_gallery::DockWindowRuntime>,
    loading_deadline: Option<Instant>,
}

impl Default for GalleryApp {
    fn default() -> Self {
        Self::new()
    }
}

impl GalleryApp {
    pub fn new() -> Self {
        Self {
            state: GalleryState::new(),
            dock_windows: HashMap::new(),
            loading_deadline: None,
        }
    }

    pub fn state(&self) -> &GalleryState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut GalleryState {
        &mut self.state
    }

    pub(crate) fn apply_message(&mut self, message: GalleryMessage) -> RuntimeProgramUpdate {
        if let GalleryMessage::WindowChrome(event) = message {
            let _ = self.state.window_chrome.update(event);
            self.sync_dock_windows();
            self.sync_loading_deadline();
            return RuntimeProgramUpdate::redraw_all();
        }
        let previous_commands = self.state.dock_window_commands.len();
        self.state.update(message);
        let window_commands = self.state.dock_window_commands[previous_commands..].to_vec();
        self.sync_dock_windows();
        self.sync_loading_deadline();
        RuntimeProgramUpdate {
            redraw: RuntimeRedraw::All,
            window_commands,
            exit: false,
        }
    }

    fn sync_dock_windows(&mut self) {
        let live = self
            .state
            .dock
            .floating
            .iter()
            .map(|surface| {
                (
                    WindowId(nana_ui::runtime::dock_surface_window_key(&surface.id)),
                    surface.clone(),
                )
            })
            .collect::<HashMap<_, _>>();
        self.dock_windows.retain(|id, _| live.contains_key(id));
        for (id, surface) in live {
            if let Some(runtime) = self.dock_windows.get_mut(&id) {
                runtime.sync(&self.state, &surface);
            } else if let Ok(runtime) =
                runtime_gallery::DockWindowRuntime::mount(&self.state, &surface)
            {
                self.dock_windows.insert(id, runtime);
            }
        }
    }

    fn sync_loading_deadline(&mut self) {
        if self.state.loading {
            if self.loading_deadline.is_none() {
                self.loading_deadline = Some(Instant::now() + LOADING_TICK);
            }
        } else {
            self.loading_deadline = None;
        }
    }

    fn persist_primary_dock(&mut self) {
        let Some(runtime) = self.state.gallery_runtime.take() else {
            return;
        };
        self.state.persist_runtime_dock_workspace(&runtime);
        self.state.gallery_runtime = Some(runtime);
        self.sync_dock_windows();
    }

    fn drain_primary_input(&mut self, event: &InputEvent) -> Vec<GalleryMessage> {
        let mut messages = if self.state.settings_open {
            self.state
                .settings_runtime
                .as_mut()
                .map(|runtime| runtime.take_host_messages(event))
                .unwrap_or_default()
        } else {
            self.state
                .gallery_runtime
                .as_mut()
                .map(|runtime| runtime.take_host_messages(event))
                .unwrap_or_default()
        };
        messages.extend(self.state.apply_overlay_host_input(event));
        if let Some(message) = shortcut_message(event) {
            messages.push(message);
        }
        messages
    }

    fn apply_all(
        &mut self,
        messages: impl IntoIterator<Item = GalleryMessage>,
    ) -> RuntimeProgramUpdate {
        let mut update = RuntimeProgramUpdate::default();
        for message in messages {
            update = merge_program_update(update, self.apply_message(message));
        }
        update
    }
}

impl RuntimeProgram for GalleryApp {
    type Message = GalleryMessage;
    type Error = FrameworkError;

    fn initialize(
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error> {
        let mut app = Self::new();
        let size = context.geometry().logical_size;
        if size.0 > 0.0 && size.1 > 0.0 {
            let _ = app.apply_message(GalleryMessage::Workspace(WorkspaceAction::WindowResized {
                width: size.0,
                height: size.1,
            }));
        }
        if app.state.gallery_runtime.is_none() {
            return Err(FrameworkError::InvalidInput);
        }
        Ok((app, Vec::new()))
    }

    fn document(&self, id: WindowId) -> Option<&nana_ui::runtime::RuntimeDocument> {
        if id == WindowId::PRIMARY {
            self.state.runtime_document()
        } else {
            self.dock_windows
                .get(&id)
                .map(runtime_gallery::DockWindowRuntime::runtime_document)
        }
    }

    fn document_mut(&mut self, id: WindowId) -> Option<&mut nana_ui::runtime::RuntimeDocument> {
        if id == WindowId::PRIMARY {
            if self.state.settings_open {
                self.state
                    .settings_runtime
                    .as_mut()
                    .map(runtime_settings::GallerySettingsRuntime::runtime_document_mut)
            } else {
                self.state
                    .gallery_runtime
                    .as_mut()
                    .map(runtime_gallery::GalleryRuntime::runtime_document_mut)
            }
        } else {
            self.dock_windows
                .get_mut(&id)
                .map(runtime_gallery::DockWindowRuntime::runtime_document_mut)
        }
    }

    fn update(
        &mut self,
        message: Self::Message,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.apply_message(message)
    }

    fn theme_mode(&self) -> ThemeMode {
        self.state.theme_mode()
    }

    fn window_material_mode(&self) -> nana_ui::MaterialEffect {
        nana_ui::window_material_effect(self.state.appearance.window_material())
    }

    fn prepare_window_frame(
        &mut self,
        id: WindowId,
        context: &RuntimeProgramContext<Self::Message>,
    ) {
        if id == WindowId::PRIMARY && self.state.material_outcome() != context.material() {
            self.state
                .update(GalleryMessage::MaterialApplied(context.material()));
        }
        if id == WindowId::PRIMARY && !self.state.settings_open {
            self.persist_primary_dock();
        }
    }

    fn input_event(
        &mut self,
        id: WindowId,
        event: &InputEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        if id == WindowId::PRIMARY {
            let messages = self.drain_primary_input(event);
            let mut update = self.apply_all(messages);
            if !self.state.settings_open {
                self.persist_primary_dock();
            }
            if !update.window_commands.is_empty() {
                update.redraw = RuntimeRedraw::All;
            }
            return Ok(update);
        }
        Ok(RuntimeProgramUpdate::redraw(id))
    }

    fn window_event(
        &mut self,
        event: WindowEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        self.handle_window_event(event)
    }

    fn next_wakeup(&self) -> Option<Instant> {
        self.loading_deadline
    }

    fn wake(
        &mut self,
        now: Instant,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        if self.state.loading
            && self
                .loading_deadline
                .is_some_and(|deadline| now >= deadline)
        {
            self.loading_deadline = None;
            return self.apply_message(GalleryMessage::LoadingTick);
        }
        RuntimeProgramUpdate::default()
    }
}

impl GalleryApp {
    pub(crate) fn handle_window_event(&mut self, event: WindowEvent) -> RuntimeProgramUpdate {
        match event {
            WindowEvent::Ready { id, geometry } | WindowEvent::Resized { id, geometry } => {
                self.apply_window_geometry(id, geometry)
            }
            WindowEvent::Moved { id, geometry } if id != WindowId::PRIMARY => {
                self.apply_window_geometry(id, geometry)
            }
            WindowEvent::CloseRequested { id } if id == WindowId::PRIMARY => {
                RuntimeProgramUpdate::exit()
            }
            WindowEvent::CloseRequested { id } => {
                if let Some(surface) = floating_surface_for_window(&self.state.dock, id) {
                    self.apply_message(GalleryMessage::Dock(GalleryDock::CloseFloating(
                        Arc::clone(&surface.id),
                    )))
                } else {
                    RuntimeProgramUpdate::default()
                }
            }
            WindowEvent::Closed { id } => {
                self.dock_windows.remove(&id);
                RuntimeProgramUpdate::default()
            }
            WindowEvent::FocusChanged { id, focused: true } if id != WindowId::PRIMARY => {
                if let Some(item) = floating_surface_for_window(&self.state.dock, id)
                    .and_then(|surface| surface.root.flatten().first().cloned())
                {
                    self.apply_message(GalleryMessage::Dock(GalleryDock::Focus(item)))
                } else {
                    RuntimeProgramUpdate::default()
                }
            }
            _ => RuntimeProgramUpdate::default(),
        }
    }

    pub(crate) fn apply_window_geometry(
        &mut self,
        id: WindowId,
        geometry: nana_ui_platform::WindowGeometry,
    ) -> RuntimeProgramUpdate {
        self.state.window_chrome.set_maximized(geometry.maximized);
        if id == WindowId::PRIMARY {
            return self.apply_message(GalleryMessage::Workspace(WorkspaceAction::WindowResized {
                width: geometry.logical_size.0,
                height: geometry.logical_size.1,
            }));
        }
        if let Some(runtime) = self.dock_windows.get_mut(&id) {
            runtime.resize(geometry.logical_size.0, geometry.logical_size.1);
        }
        if let Some((surface_id, fallback_x, fallback_y)) =
            floating_surface_for_window(&self.state.dock, id)
                .map(|surface| (Arc::clone(&surface.id), surface.x, surface.y))
        {
            let (x, y) = geometry
                .logical_position
                .unwrap_or((fallback_x, fallback_y));
            // Platform-originated geometry must not echo WindowCommand::Move.
            self.state.dock.apply(DockWorkspaceEvent::MoveFloating {
                id: surface_id,
                x,
                y,
                width: geometry.logical_size.0,
                height: geometry.logical_size.1,
            });
        }
        RuntimeProgramUpdate::redraw(id)
    }
}

fn shortcut_message(event: &InputEvent) -> Option<GalleryMessage> {
    let InputEvent::Keyboard {
        pressed: true,
        key,
        modifiers,
        ..
    } = event
    else {
        return None;
    };
    if !modifiers.meta && !modifiers.control {
        return None;
    }
    Some(GalleryMessage::KeyStroke(KeyStroke::new(
        key,
        KeyModifiers {
            control: modifiers.control,
            alt: modifiers.alt,
            shift: modifiers.shift,
            logo: modifiers.meta,
        },
    )))
}

fn merge_program_update(
    mut left: RuntimeProgramUpdate,
    right: RuntimeProgramUpdate,
) -> RuntimeProgramUpdate {
    left.exit |= right.exit;
    left.window_commands.extend(right.window_commands);
    left.redraw = match (left.redraw, right.redraw) {
        (RuntimeRedraw::All, _) | (_, RuntimeRedraw::All) => RuntimeRedraw::All,
        (RuntimeRedraw::None, redraw) | (redraw, RuntimeRedraw::None) => redraw,
        (RuntimeRedraw::Window(first), RuntimeRedraw::Window(second)) if first == second => {
            RuntimeRedraw::Window(first)
        }
        _ => RuntimeRedraw::All,
    };
    left
}

const FLOATING_MIN_WIDTH: f64 = 160.0;
const FLOATING_MIN_HEIGHT: f64 = 120.0;
const DOCK_WINDOW_TITLE: &str = "NanaUI Gallery";
const DOCK_CENTER: &str = "gallery.primary";

fn gallery_dock_allowed_when_locked(action: &GalleryDock) -> bool {
    matches!(
        action,
        GalleryDock::SetLocked(_)
            | GalleryDock::Focus(_)
            | GalleryDock::ActivateTab(_)
            | GalleryDock::MoveFloating { .. }
    )
}

fn floating_surface_for_window(
    workspace: &DockWorkspace,
    window_id: WindowId,
) -> Option<&nana_ui::runtime::DockFloatingSurface> {
    workspace
        .floating
        .iter()
        .find(|surface| nana_ui::runtime::dock_surface_window_key(&surface.id) == window_id.0)
}

fn floating_surface_for_item(workspace: &DockWorkspace, id: &str) -> Option<Arc<str>> {
    workspace
        .floating
        .iter()
        .find(|surface| surface.root.contains(id))
        .map(|surface| Arc::clone(&surface.id))
}

fn activate_runtime_dock_tab_in_workspace(workspace: &mut DockWorkspace, id: &str) {
    if activate_runtime_dock_tab(&mut workspace.main, id) {
        return;
    }
    for surface in &mut workspace.floating {
        if activate_runtime_dock_tab(&mut surface.root, id) {
            return;
        }
    }
}

fn activate_runtime_dock_tab(node: &mut nana_ui::runtime::DockNode, id: &str) -> bool {
    match node {
        nana_ui::runtime::DockNode::Item { .. } => false,
        nana_ui::runtime::DockNode::Tabs { tabs, active, .. } => {
            if tabs.iter().any(|tab| tab.as_ref() == id) && active.as_ref() != id {
                *active = Arc::from(id);
                true
            } else {
                false
            }
        }
        nana_ui::runtime::DockNode::Split { first, second, .. } => {
            activate_runtime_dock_tab(first, id) || activate_runtime_dock_tab(second, id)
        }
    }
}

/// Maps Runtime floating-dock events the same way as
/// `nana_ui::runtime_dock_window_update` (hosted). Gallery records the
/// commands; the Scene host does not open extra daemon windows.
fn runtime_dock_window_commands(
    events: impl IntoIterator<Item = DockWorkspaceEvent>,
) -> Vec<WindowCommand> {
    events
        .into_iter()
        .map(|event| match event {
            DockWorkspaceEvent::OpenFloating(surface) => WindowCommand::Open {
                id: WindowId(nana_ui::runtime::dock_surface_window_key(&surface.id)),
                settings: WindowSettings {
                    title: DOCK_WINDOW_TITLE.to_owned(),
                    initial_size: (f64::from(surface.width), f64::from(surface.height)),
                    minimum_size: (FLOATING_MIN_WIDTH, FLOATING_MIN_HEIGHT),
                    initial_position: Some((f64::from(surface.x), f64::from(surface.y))),
                    maximized: false,
                    transparent: false,
                    always_on_top: false,
                    resizable: true,
                    role: WindowRole::Tool,
                    modal: false,
                    parent: None,
                    system_caption: true,
                },
            },
            DockWorkspaceEvent::CloseFloating(id) => {
                WindowCommand::Close(WindowId(nana_ui::runtime::dock_surface_window_key(&id)))
            }
            DockWorkspaceEvent::MoveFloating { id, x, y, .. } => WindowCommand::Move {
                id: WindowId(nana_ui::runtime::dock_surface_window_key(&id)),
                position: (x, y),
            },
            DockWorkspaceEvent::FocusFloating(id) => {
                WindowCommand::Focus(WindowId(nana_ui::runtime::dock_surface_window_key(&id)))
            }
        })
        .collect()
}

fn gallery_dock_workspace() -> DockWorkspace {
    use nana_ui::runtime::{DockAxis, DockNode};

    let main = DockNode::split(
        DockAxis::Horizontal,
        0.26,
        DockNode::tabs(
            ["gallery.navigation", "gallery.assets"],
            "gallery.navigation",
            [("gallery.navigation", None), ("gallery.assets", None)],
        ),
        DockNode::split(
            DockAxis::Vertical,
            0.68,
            DockNode::split(
                DockAxis::Horizontal,
                0.72,
                DockNode::item("gallery.primary", None),
                DockNode::tabs(
                    ["gallery.inspector", "gallery.outline"],
                    "gallery.inspector",
                    [("gallery.inspector", None), ("gallery.outline", None)],
                ),
            ),
            DockNode::tabs(
                ["gallery.console", "gallery.problems", "gallery.output"],
                "gallery.console",
                [
                    ("gallery.console", None),
                    ("gallery.problems", None),
                    ("gallery.output", None),
                ],
            ),
        ),
    );
    DockWorkspace::new(main).primary(DOCK_CENTER)
}

fn gallery_layout(show_workspace: bool) -> WorkspaceLayout {
    WorkspaceLayout::new([
        RegionState::new(RegionId::Resources, RegionRole::Resources)
            .size(220.0)
            .min_size(180.0)
            .max_size(480.0)
            .collapsible(true)
            .resizable(true),
        RegionState::new(RegionId::PrimaryToolbar, RegionRole::Utility)
            .placement(RegionPlacement::Top)
            .scope(RegionScope::Primary)
            .size(34.0)
            .hidden(!show_workspace),
        RegionState::new(RegionId::Primary, RegionRole::Primary)
            .min_size(320.0)
            .fill_priority(1),
        RegionState::new(RegionId::Inspector, RegionRole::Inspector)
            .size(280.0)
            .min_size(200.0)
            .max_size(560.0)
            .collapsible(true)
            .resizable(true)
            .hidden(!show_workspace),
        RegionState::new(RegionId::Diagnostics, RegionRole::Utility)
            .placement(RegionPlacement::Bottom)
            .scope(RegionScope::Primary)
            .size(180.0)
            .min_size(96.0)
            .max_size(420.0)
            .collapsible(true)
            .resizable(true)
            .hidden(!show_workspace),
    ])
    .expect("gallery workspace region ids are unique")
}

#[allow(dead_code)]
fn gallery_calendar_model() -> CalendarHeatmapModel {
    build_calendar_heatmap_model(
        &(0..84)
            .map(|offset| {
                let day = 1 + offset;
                let month = 4 + (day - 1) / 30;
                let day_of_month = 1 + (day - 1) % 30;
                CalendarHeatmapDatum::<()>::new(
                    format!("2026-{month:02}-{day_of_month:02}"),
                    ((offset * 7 + 3) % 18) as f32,
                )
            })
            .collect::<Vec<_>>(),
        CalendarHeatmapOptions::default(),
    )
}

fn gallery_context_items() -> Vec<ContextMenuItem> {
    vec![
        ContextMenuItem::new("project", "项目操作"),
        ContextMenuItem::new("project/duplicate", "复制项目").icon(Icon::Add),
        ContextMenuItem::new("project/rename", "重命名项目").icon(Icon::File),
        ContextMenuItem::new("remove", "移除项目")
            .icon(Icon::Close)
            .danger(true),
    ]
}

fn context_action_from_value(value: &str) -> Option<ContextAction> {
    let key = value.rsplit('/').next().unwrap_or(value);
    match key {
        "duplicate" => Some(ContextAction::Duplicate),
        "rename" => Some(ContextAction::Rename),
        "remove" => Some(ContextAction::Remove),
        _ => None,
    }
}

fn gallery_action_registry() -> ActionRegistry {
    let mut registry = ActionRegistry::new();
    for action in [
        ActionDescriptor::new("workspace.command_palette", "显示命令面板")
            .category("工作区")
            .keywords(["command", "palette"]),
        ActionDescriptor::new("appearance.toggle_theme", "切换深浅主题")
            .category("外观")
            .keywords(["theme", "dark", "light"]),
        ActionDescriptor::new("workspace.toggle_sidebar", "切换侧栏")
            .category("工作区")
            .keywords(["sidebar", "navigation"]),
        ActionDescriptor::new("workspace.reset_layout", "恢复工作区布局")
            .category("工作区")
            .keywords(["layout", "reset"]),
        ActionDescriptor::new("graph.reset_viewport", "重置节点图视口")
            .category("节点图")
            .keywords(["graph", "viewport"])
            .when(ContextPredicate::always().all_of(["graph"])),
    ] {
        registry
            .register(action)
            .expect("gallery action ids are unique");
    }
    registry
}

fn gallery_keymap() -> Keymap {
    Keymap::new([
        KeyBinding::new(
            "workspace.command_palette",
            KeyStroke::new("p", KeyModifiers::primary().with_shift()),
        ),
        KeyBinding::new(
            "appearance.toggle_theme",
            KeyStroke::new("t", KeyModifiers::primary().with_shift()),
        ),
    ])
}

fn settings_layout() -> WorkspaceLayout {
    WorkspaceLayout::new([
        RegionState::new(RegionId::Resources, RegionRole::Resources)
            .size(220.0)
            .min_size(180.0)
            .max_size(480.0)
            .collapsible(true)
            .resizable(true),
        RegionState::new(RegionId::Primary, RegionRole::Primary)
            .min_size(420.0)
            .fill_priority(1),
    ])
    .expect("gallery settings region ids are unique")
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
    .expect("gallery settings model is valid")
}

fn appearance_message(event: AppearanceEvent) -> GalleryMessage {
    match event {
        AppearanceEvent::Theme(theme) => GalleryMessage::SetTheme(theme),
        AppearanceEvent::StandardRadius(radius) => GalleryMessage::SetStandardRadius(radius),
        AppearanceEvent::WorkspaceCorners(enabled) => GalleryMessage::SetWorkspaceCorners(enabled),
        AppearanceEvent::WindowMaterial(mode) => GalleryMessage::SetWindowMaterial(mode),
        AppearanceEvent::BackdropTarget(target) => GalleryMessage::SetBackdropTarget(target),
        AppearanceEvent::BackdropOpacity(opacity) => GalleryMessage::SetBackdropOpacity(opacity),
        AppearanceEvent::TitlebarFollowsSidebar(enabled) => {
            GalleryMessage::SetTitlebarFollowsSidebar(enabled)
        }
        AppearanceEvent::Reset => GalleryMessage::ResetAppearance,
    }
}

pub(crate) fn section_label(section: GallerySection) -> &'static str {
    match section {
        GallerySection::Controls => "控件",
        GallerySection::Surfaces => "表面",
        GallerySection::Feedback => "反馈",
        GallerySection::RichText => "富文本",
        GallerySection::Graph => "节点图",
        GallerySection::Workspace => "工作区",
    }
}

const MARKDOWN_FIXTURE: &str = r#"# 原生 Markdown

NanaUI 使用 **Rust 解析** 与 *WGPU 向量渲染*，支持[链接](https://example.com)、`inline code` 和任务列表。

行内公式 $E = mc^2$ 与正文共享布局流。

- [x] CommonMark / GFM
- [x] KaTeX 字形布局
- [x] Mermaid 无浏览器渲染

| 能力 | 路径 | 状态 |
| :--- | :---: | ---: |
| 数学公式 | RaTeX | Ready |
| 图表 | Merman | Ready |

$$\frac{-b \pm \sqrt{b^2-4ac}}{2a}$$

```mermaid
flowchart LR
  Markdown --> Parse[Rust AST]
  Parse --> Math[KaTeX layout]
  Parse --> Diagram[Mermaid layout]
  Math --> WGPU
  Diagram --> WGPU
```

```rust
let document = NativeMarkdown::parse(source);
```
"#;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
