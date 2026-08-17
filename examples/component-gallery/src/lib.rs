use std::cell::OnceCell;
use std::sync::Arc;

use iced::widget::{
    button, container, row, scrollable, slider, space, stack, text, text_editor, toggler,
};
use iced::{Alignment, Element, Length, Padding, Point, Size, Subscription, Task, font};

use nana_ui::command::{
    ActionDescriptor, ActionId, ActionPickerNavigation, ActionPickerState, ActionRegistry,
    ContextPredicate, KeyBinding, KeyContext, KeyModifiers, KeyStroke, Keymap, KeymapMatch,
    KeymapState,
};
use nana_ui::compatibility::{
    AppTitleBar, PaneChrome, PaneChromeAction, PaneChromeActionKind, PaneTree, PaneTreeNode,
};
use nana_ui::compatibility::{
    Button as UiButton, Card as UiCard, Checkbox as UiCheckbox, ConfirmDialog as UiConfirmDialog,
    IconButton as UiIconButton, Input as UiInput, InteractiveCard as UiInteractiveCard,
    ListItem as UiListItem, Popover as UiPopover, Progress as UiProgress,
    RangeField as UiRangeField, SegmentedControl as UiSegmentedControl, Switch as UiSwitch,
    Tabs as UiTabs, Textarea as UiTextarea, Tooltip as UiTooltip, XYPad as UiXYPad,
};
use nana_ui::compatibility::{
    CalendarHeatmap as UiCalendarHeatmap, CalendarHeatmapActiveCell, CalendarHeatmapDatum,
    CalendarHeatmapEvent, CalendarHeatmapModel, CalendarHeatmapOptions, GraphCanvas,
    ImageViewer as UiImageViewer, ImageViewerSource, NativeMarkdown, build_calendar_heatmap_model,
};
use nana_ui::compatibility::{
    CommandPalette as UiCommandPalette, Dropdown as UiDropdown, DropdownOption,
    SearchDropdown as UiSearchDropdown, SearchDropdownOption, SearchDropdownState,
    TreeView as UiTreeView,
};
use nana_ui::components::{
    AnchoredMenuPlacement, AnchoredMenuPosition, CommandPaletteEvent, CommandPaletteItem,
    ContextMenuAnchor, ContextMenuEvent, ContextMenuHost, ContextMenuItem, ContextMenuTrigger,
    ControlSize, DropdownEvent, SelectionOption, TreeNode, TreeViewEvent, XYPadEvent, XYPadValue,
};
use nana_ui::dialog::{DialogClosePolicy, DialogCloseTrigger, DialogSize};
use nana_ui::icons::{Icon, icon, status_indicator};
use nana_ui::layout::{
    RegionId, RegionPlacement, RegionRole, RegionScope, RegionState, WorkspaceLayout,
};
use nana_ui::menu::{MenuConfirmation, MenuSelection};
use nana_ui::overlay::ExclusiveOverlay;
use nana_ui::selection::{SelectionMove, SingleSelection};
use nana_ui::settings::{
    AppearanceSettings, BackdropTarget, SettingsModel, SettingsState, SettingsTab, SettingsTabId,
    WindowMaterialMode,
};
use nana_ui::theme::{Colors, ThemeMode, ThemeModeExt, ThemeTokens, UI_METRICS, ui_font};
use nana_ui::tooltip::TooltipConfig;
use nana_ui::widgets::{
    ButtonKind, CardKind, button_style, canvas_style, panel_style, scrollable_style, slider_style,
    toggler_style, toolbar_style, vertical_scrollbar,
};
use nana_ui::window_chrome::{WindowChromeEvent, WindowChromeState};
use nana_ui::workspace::{WorkspaceAction, WorkspaceController};
use nana_ui::{
    AppearanceEvent, DockAction, DockId, DockSurfaceId, DockWorkspace, DockWorkspaceEvent,
    FallbackColor, GraphCanvasEvent, GraphEdge, GraphEndpoint, GraphModel, GraphNode, GraphPoint,
    GraphPort, GraphPortKind, GraphPortSide, GraphSelection, GraphSize, GraphViewport,
    MaterialOutcome, PopupShell, PopupTitleBarFrame, SplitAxis, SplitPaneAction,
    SplitPaneController, WindowAppearance, apply_system_material, clear_system_material,
    ratio_pane_split,
};
use nana_ui_platform::{WindowCommand, WindowId, WindowRole, WindowSettings};

#[path = "views/controls.rs"]
mod controls_view;
#[path = "views/feedback.rs"]
mod feedback_view;
#[path = "views/graph.rs"]
mod graph_view;
#[path = "views/rich_text.rs"]
mod rich_text_view;
#[path = "views/root.rs"]
mod root_view;
mod runtime_gallery;
mod runtime_host;
mod runtime_overlays;
mod runtime_settings;
#[path = "views/settings.rs"]
mod settings_view;
#[path = "views/surfaces.rs"]
mod surfaces_view;
#[path = "views/workspace.rs"]
mod workspace_view;

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

#[derive(Debug, Clone, PartialEq)]
pub enum GalleryMessage {
    Workspace(WorkspaceAction),
    SplitPane(SplitPaneAction),
    Dock(DockAction),
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
    EditText(text_editor::Action),
    ToggleContextMenu,
    OpenContextMenu(ContextMenuAnchor),
    ToggleImageViewer,
    RequestImageViewerClose(DialogCloseTrigger),
    TogglePopover,
    ClosePopover,
    ContextMenu(ContextMenuEvent<ContextAction>),
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

fn overlay_event(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<GalleryMessage> {
    let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, .. }) = event else {
        return None;
    };
    nana_ui::action_picker_from_iced_key(&key).map(GalleryMessage::NavigateCommandPalette)
}

fn command_shortcut_event(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<GalleryMessage> {
    let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) = event
    else {
        return None;
    };
    if !modifiers.command() {
        return None;
    }
    KeyStroke::from_iced(&key, modifiers).map(GalleryMessage::KeyStroke)
}

#[allow(dead_code)]
fn surface_selection_event(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<GalleryMessage> {
    let movement = match event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(named),
            ..
        }) => match named {
            iced::keyboard::key::Named::ArrowLeft => SelectionMove::Previous,
            iced::keyboard::key::Named::ArrowRight => SelectionMove::Next,
            iced::keyboard::key::Named::Home => SelectionMove::First,
            iced::keyboard::key::Named::End => SelectionMove::Last,
            _ => return None,
        },
        _ => return None,
    };
    Some(GalleryMessage::NavigateSurfaceView(movement))
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
    search_dropdown: SearchDropdownState<u8>,
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
    context_anchor: Option<ContextMenuAnchor>,
    context_items: OnceCell<Vec<ContextMenuItem<'static, ContextAction>>>,
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
    editor: text_editor::Content,
    primary_clicks: u32,
    window_chrome: WindowChromeState,
    /// Latest material application outcome from the iced host path.
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
            search_dropdown: SearchDropdownState::new([
                SearchDropdownOption::new(1, "第一个选项").hint("Alpha"),
                SearchDropdownOption::new(2, "第二个选项").hint("Beta"),
                SearchDropdownOption::new(3, "第三个选项").hint("Gamma"),
            ]),
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
            editor: text_editor::Content::with_text("示例说明\n用于展示多行文本编辑"),
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

    fn context_items(&self) -> &[ContextMenuItem<'static, ContextAction>] {
        self.context_items
            .get_or_init(gallery_context_items)
            .as_slice()
    }

    pub fn subscription(&self) -> Subscription<GalleryMessage> {
        let interaction = if self.overlay.is_open() {
            Subscription::batch([
                iced::event::listen_with(overlay_event),
                iced::event::listen_with(runtime_overlays::overlay_runtime_key_event),
            ])
        } else if self.settings_open {
            iced::event::listen_with(runtime_settings::settings_runtime_key_event)
        } else {
            iced::event::listen_with(runtime_gallery::gallery_runtime_key_event)
        };
        let loading = if self.loading {
            iced::time::every(iced::time::Duration::from_millis(100))
                .map(|_| GalleryMessage::LoadingTick)
        } else {
            Subscription::none()
        };
        Subscription::batch([
            iced::event::listen_with(command_shortcut_event),
            interaction,
            loading,
            self.active_workspace()
                .subscription()
                .map(GalleryMessage::Workspace),
            self.split_pane
                .subscription()
                .map(GalleryMessage::SplitPane),
            WindowChromeState::subscription().map(GalleryMessage::WindowChrome),
        ])
    }

    pub fn update_windowed(&mut self, message: GalleryMessage) -> Task<GalleryMessage> {
        if let GalleryMessage::MaterialApplied(outcome) = message {
            self.material_outcome = outcome;
            return Task::none();
        }

        if let GalleryMessage::WindowChrome(event) = message {
            let reapply = matches!(event, WindowChromeEvent::PrepareWindow(_));
            let chrome = self
                .window_chrome
                .update_iced(event)
                .map(GalleryMessage::WindowChrome);
            if reapply {
                return Task::batch([chrome, self.apply_window_material_task()]);
            }
            return chrome;
        }

        let focus_palette = !self.action_picker.is_open();
        let previous_theme = self.theme;
        let refresh_material = matches!(
            message,
            GalleryMessage::SetWindowMaterial(_)
                | GalleryMessage::ResetAppearance
                | GalleryMessage::SetTheme(_)
                | GalleryMessage::ToggleTheme
        );
        self.update(message);
        if focus_palette && self.action_picker.is_open() {
            iced::widget::operation::focus(nana_ui::COMMAND_PALETTE_INPUT_ID)
        } else if refresh_material || self.theme != previous_theme {
            self.apply_window_material_task()
        } else {
            Task::none()
        }
    }

    fn apply_window_material_task(&self) -> Task<GalleryMessage> {
        let Some(window_id) = self.window_chrome.window_id() else {
            return Task::none();
        };
        let theme = self.theme;
        let mode = self.appearance.window_material();
        iced::window::run(window_id, move |window| {
            apply_gallery_window_material(window, theme, mode)
        })
        .map(GalleryMessage::MaterialApplied)
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
                self.editor = text_editor::Content::with_text(&value);
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
            GalleryMessage::Dock(action) => self.apply_dock_action(action),
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
                self.overlay.dismiss();
                self.menu_confirmation.clear();
            }
            GalleryMessage::OverlayInteraction => {}
            GalleryMessage::EditText(action) => self.editor.perform(action),
            GalleryMessage::ToggleContextMenu => {
                self.menu_confirmation.clear();
                self.context_query.clear();
                self.context_path.clear();
                self.context_anchor = None;
                self.overlay.toggle(GalleryOverlay::ContextMenu);
            }
            GalleryMessage::OpenContextMenu(anchor) => {
                self.menu_confirmation.clear();
                self.context_query.clear();
                self.context_path.clear();
                self.context_anchor = Some(anchor);
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
                ContextMenuEvent::Search(query) => {
                    self.context_query = query;
                    self.context_path.clear();
                }
                ContextMenuEvent::OpenSubmenu(path) => self.context_path = path,
                ContextMenuEvent::Select(action) => {
                    if !self.overlay.contains(&GalleryOverlay::ContextMenu) {
                        return;
                    }
                    let requires_confirmation = action == ContextAction::Remove;
                    if let MenuSelection::Confirmed(action) =
                        self.menu_confirmation.select(action, requires_confirmation)
                    {
                        self.apply_context_action(action);
                    }
                }
                ContextMenuEvent::Dismiss => {
                    self.overlay.dismiss();
                    self.context_anchor = None;
                    self.menu_confirmation.clear();
                    self.context_query.clear();
                    self.context_path.clear();
                }
                ContextMenuEvent::Interaction => {}
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

    fn apply_dock_action(&mut self, action: DockAction) {
        if self.dock_locked && !dock_action_allowed_when_locked(&action) {
            return;
        }
        match action {
            DockAction::ActivateTab(id) => {
                activate_runtime_dock_tab_in_workspace(&mut self.dock, id.as_str());
            }
            DockAction::SurfaceResized {
                surface,
                width,
                height,
            } => {
                if let Some(id) = floating_surface_id(surface) {
                    let Some(existing) = self
                        .dock
                        .floating
                        .iter()
                        .find(|item| item.id == id)
                        .cloned()
                    else {
                        return;
                    };
                    self.apply_dock_workspace_event(DockWorkspaceEvent::MoveFloating {
                        id,
                        x: existing.x,
                        y: existing.y,
                        width,
                        height,
                    });
                }
            }
            DockAction::SurfaceGeometry { surface, bounds }
            | DockAction::SurfaceLayout { surface, bounds } => {
                if let Some(id) = floating_surface_id(surface) {
                    if self.dock.floating.iter().any(|item| item.id == id) {
                        self.apply_dock_workspace_event(DockWorkspaceEvent::MoveFloating {
                            id,
                            x: bounds.x,
                            y: bounds.y,
                            width: bounds.width,
                            height: bounds.height,
                        });
                    }
                }
            }
            DockAction::Hide(id) => {
                let _ = self.dock.hide(id.as_str());
            }
            DockAction::Show(id) => {
                let _ = self.dock.show(id.as_str());
            }
            DockAction::Float { id, bounds, .. } => {
                if id.as_str() == DOCK_CENTER {
                    return;
                }
                let Some(event) = self.dock.float_item_at(
                    id.as_str(),
                    bounds.x,
                    bounds.y,
                    bounds.width,
                    bounds.height,
                ) else {
                    return;
                };
                self.record_dock_workspace_events([event]);
            }
            DockAction::Focus(id) => {
                activate_runtime_dock_tab_in_workspace(&mut self.dock, id.as_str());
                if let Some(surface) = floating_surface_for_item(&self.dock, id.as_str()) {
                    self.apply_dock_workspace_event(DockWorkspaceEvent::FocusFloating(surface));
                }
            }
            DockAction::CloseSurface(surface) => {
                if let Some(id) = floating_surface_id(surface) {
                    if self.dock.floating.iter().any(|item| item.id == id) {
                        self.apply_dock_workspace_event(DockWorkspaceEvent::CloseFloating(id));
                    }
                }
            }
            DockAction::SetLocked(locked) => {
                self.dock_locked = locked;
            }
            DockAction::Reset => {
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
            _ => {}
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
                self.editor = text_editor::Content::with_text("已重命名项目");
            }
            ContextAction::Remove => {
                self.selected_item = 0;
            }
        }
    }
}

const FLOATING_MIN_WIDTH: f64 = 160.0;
const FLOATING_MIN_HEIGHT: f64 = 120.0;
const DOCK_WINDOW_TITLE: &str = "NanaUI Gallery";
const DOCK_CENTER: &str = "gallery.primary";

fn dock_action_allowed_when_locked(action: &DockAction) -> bool {
    matches!(
        action,
        DockAction::SetLocked(_)
            | DockAction::Focus(_)
            | DockAction::ActivateTab(_)
            | DockAction::SurfaceResized { .. }
            | DockAction::SurfaceGeometry { .. }
            | DockAction::SurfaceLayout { .. }
            | DockAction::CardHover(..)
    )
}

fn floating_surface_id(surface: DockSurfaceId) -> Option<Arc<str>> {
    (surface.0 != 0).then(|| Arc::<str>::from(surface.0.to_string()))
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
/// commands; the Iced host does not open extra daemon windows.
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

#[allow(dead_code)]
fn section_heading<'a, Message>(
    title: &'a str,
    trailing: Option<Element<'a, Message>>,
    colors: Colors,
) -> Element<'a, Message>
where
    Message: 'a,
{
    let mut content = row![
        text(title)
            .size(12)
            .font(ui_font(font::Weight::Bold))
            .color(colors.muted)
    ]
    .align_y(Alignment::Center)
    .spacing(8);
    if let Some(trailing) = trailing {
        content = content.push(space().width(Length::Fill)).push(trailing);
    }
    container(content)
        .height(Length::Fixed(ControlSize::Small.height()))
        .padding([0.0, UI_METRICS.selection_padding_x])
        .align_y(iced::alignment::Vertical::Center)
        .into()
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

fn gallery_context_items() -> Vec<ContextMenuItem<'static, ContextAction>> {
    vec![
        ContextMenuItem::new(ContextAction::Duplicate, "项目操作").children([
            ContextMenuItem::new(ContextAction::Duplicate, "复制项目")
                .icon(Icon::Add)
                .keywords(["copy", "duplicate"]),
            ContextMenuItem::new(ContextAction::Rename, "重命名项目")
                .icon(Icon::File)
                .keywords(["edit", "name"]),
        ]),
        ContextMenuItem::new(ContextAction::Remove, "移除项目")
            .icon(Icon::Close)
            .keywords(["delete", "remove"])
            .confirm_label("再次点击确认移除")
            .danger(true),
    ]
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

fn apply_gallery_window_material(
    window: &dyn iced::Window,
    theme: ThemeMode,
    mode: WindowMaterialMode,
) -> MaterialOutcome {
    match mode {
        WindowMaterialMode::Solid => {
            clear_system_material(window);
            MaterialOutcome::chosen_solid()
        }
        WindowMaterialMode::Translucent => {
            let (appearance, fallback) = match theme {
                ThemeMode::Dark => (WindowAppearance::Dark, FallbackColor::rgba(24, 24, 24, 220)),
                ThemeMode::Light => (
                    WindowAppearance::Light,
                    FallbackColor::rgba(255, 255, 255, 232),
                ),
            };
            apply_system_material(window, appearance, fallback)
        }
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
