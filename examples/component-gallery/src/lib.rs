use std::cell::OnceCell;

use iced::widget::{
    button, container, row, scrollable, slider, space, stack, text, text_editor, toggler,
};
use iced::{Alignment, Element, Length, Point, Size, Subscription, Task, font};

use nana_ui::components::{
    AboutMetadata, AboutSection, AnchoredMenuPlacement, AnchoredMenuPosition, AppearanceEvent,
    AppearanceSection, Button as UiButton, CalendarHeatmap as UiCalendarHeatmap,
    CalendarHeatmapActiveCell, CalendarHeatmapDatum, CalendarHeatmapEvent, CalendarHeatmapModel,
    CalendarHeatmapOptions, Card as UiCard, Checkbox as UiCheckbox,
    ConfirmDialog as UiConfirmDialog, ContextMenuEvent, ContextMenuHost, ContextMenuItem,
    ControlSize, Dropdown as UiDropdown, DropdownEvent, DropdownOption, IconButton as UiIconButton,
    ImageViewer as UiImageViewer, ImageViewerSource, Input as UiInput,
    InteractiveCard as UiInteractiveCard, ListItem as UiListItem, OverlayHost,
    Popover as UiPopover, Progress as UiProgress, RangeField as UiRangeField,
    SearchDropdown as UiSearchDropdown, SearchDropdownOption, SearchDropdownState,
    SegmentedControl as UiSegmentedControl, SelectionOption, SettingsCollapsibleCard,
    Switch as UiSwitch, Tabs as UiTabs, Textarea as UiTextarea, Tooltip as UiTooltip,
    XYPad as UiXYPad, XYPadEvent, XYPadValue,
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
    AppearanceSettings, SettingsModel, SettingsState, SettingsTab, SettingsTabId, settings_page,
    settings_sidebar as settings_sidebar_view,
};
use nana_ui::sidebar::{
    SidebarFooter, SidebarFooterButton, SidebarFrame, SidebarRow, SidebarRowState, SidebarSection,
};
use nana_ui::theme::{Colors, ThemeMode, ThemeTokens, UI_METRICS, ui_font};
use nana_ui::tooltip::TooltipConfig;
use nana_ui::widgets::{
    ButtonKind, CardKind, button_style, canvas_style, panel_style, scrollable_style, slider_style,
    toggler_style, toolbar_style, vertical_scrollbar,
};
use nana_ui::window_chrome::{WindowChromeEvent, WindowChromeState};
use nana_ui::workspace::{WorkspaceAction, WorkspaceController};
use nana_ui::{
    AppTitleBar, DesktopShell, DockAction, DockAxis, DockContents, DockController, DockHostEffect,
    DockId, DockItemSpec, DockLayout, DockNode, DockSurfaceId, PopupShell, PopupTitleBarFrame,
    SplitAxis, SplitPaneAction, SplitPaneController, dock_workspace,
};

#[path = "views/controls.rs"]
mod controls_view;
#[path = "views/feedback.rs"]
mod feedback_view;
#[path = "views/root.rs"]
mod root_view;
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
    ResetAppearance,
    SelectSection(GallerySection),
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
    CalendarHeatmap(CalendarHeatmapEvent),
    SelectListItem(usize),
    SelectSurfaceCard(usize),
    SelectSurfaceView(SurfaceView),
    NavigateSurfaceView(SelectionMove),
    ToggleDialog,
    ConfirmDialog,
    RequestDialogClose(DialogCloseTrigger),
    DismissOverlay,
    OverlayInteraction,
    EditText(text_editor::Action),
    ToggleContextMenu,
    ToggleImageViewer,
    RequestImageViewerClose(DialogCloseTrigger),
    TogglePopover,
    ClosePopover,
    ContextMenu(ContextMenuEvent<ContextAction>),
    WindowChrome(WindowChromeEvent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextAction {
    Duplicate,
    Rename,
    Remove,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GalleryOverlay {
    ContextMenu,
    Dialog,
    ImageViewer,
}

fn overlay_event(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<GalleryMessage> {
    match event {
        iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
            ..
        }) => Some(GalleryMessage::RequestDialogClose(
            DialogCloseTrigger::Escape,
        )),
        _ => None,
    }
}

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
    dock: DockController,
    dock_effects: Vec<DockHostEffect>,
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
    search_selection: Option<u8>,
    calendar_model: OnceCell<CalendarHeatmapModel>,
    calendar_active: Option<CalendarHeatmapActiveCell>,
    selected_item: usize,
    selected_surface_card: usize,
    surface_selection: SingleSelection,
    overlay: ExclusiveOverlay<GalleryOverlay>,
    dialog_policy: DialogClosePolicy,
    menu_confirmation: MenuConfirmation<ContextAction>,
    context_action: Option<ContextAction>,
    context_items: OnceCell<Vec<ContextMenuItem<'static, ContextAction>>>,
    context_query: String,
    context_path: Vec<usize>,
    popover_open: bool,
    confirmed_actions: u32,
    editor: text_editor::Content,
    primary_clicks: u32,
    window_chrome: WindowChromeState,
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
        Self {
            theme: ThemeMode::Dark,
            appearance: AppearanceSettings::default(),
            workspace: WorkspaceController::with_layout(gallery_layout(false)),
            settings_workspace: WorkspaceController::with_layout(settings_layout()),
            split_pane: SplitPaneController::new(SplitAxis::Vertical, 120.0, 64.0, 280.0),
            dock: gallery_dock(),
            dock_effects: Vec::new(),
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
            search_selection: Some(2),
            calendar_model: OnceCell::new(),
            calendar_active: None,
            selected_item: 0,
            selected_surface_card: 0,
            surface_selection: SingleSelection::new(0),
            overlay: ExclusiveOverlay::new(),
            dialog_policy: DialogClosePolicy::default(),
            menu_confirmation: MenuConfirmation::new(),
            context_action: None,
            context_items: OnceCell::new(),
            context_query: String::new(),
            context_path: Vec::new(),
            popover_open: false,
            confirmed_actions: 0,
            editor: text_editor::Content::with_text("示例说明\n用于展示多行文本编辑"),
            primary_clicks: 0,
            window_chrome: WindowChromeState::default(),
        }
    }

    pub fn theme_mode(&self) -> ThemeMode {
        self.theme
    }

    fn theme_tokens(&self) -> ThemeTokens {
        ThemeTokens::new(self.theme.colors(), self.appearance.metrics())
            .with_workspace_corners(self.appearance.workspace_corners_enabled())
    }

    fn editor_enabled(&self) -> bool {
        self.checked && self.switched
    }

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
            iced::event::listen_with(overlay_event)
        } else if !self.settings_open && self.section == GallerySection::Surfaces {
            iced::event::listen_with(surface_selection_event)
        } else {
            Subscription::none()
        };
        let loading = if self.loading {
            iced::time::every(iced::time::Duration::from_millis(100))
                .map(|_| GalleryMessage::LoadingTick)
        } else {
            Subscription::none()
        };
        Subscription::batch([
            interaction,
            loading,
            self.active_workspace()
                .subscription()
                .map(GalleryMessage::Workspace),
            self.split_pane
                .subscription()
                .map(GalleryMessage::SplitPane),
            self.dock.subscription().map(GalleryMessage::Dock),
            WindowChromeState::subscription().map(GalleryMessage::WindowChrome),
        ])
    }

    pub fn update_windowed(&mut self, message: GalleryMessage) -> Task<GalleryMessage> {
        if let GalleryMessage::WindowChrome(event) = message {
            return self
                .window_chrome
                .update_iced(event)
                .map(GalleryMessage::WindowChrome);
        }
        self.update(message);
        Task::none()
    }

    pub fn update(&mut self, message: GalleryMessage) {
        match message {
            GalleryMessage::WindowChrome(event) => {
                self.window_chrome.update(event);
            }
            GalleryMessage::Workspace(action) => {
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
            GalleryMessage::Dock(action) => {
                let update = self.dock.update(action);
                self.dock_effects.extend(update.effects);
            }
            GalleryMessage::ToggleTheme => self.theme = self.theme.toggle(),
            GalleryMessage::SetTheme(theme) => self.theme = theme,
            GalleryMessage::SetStandardRadius(radius) => {
                self.appearance.set_standard_radius(f32::from(radius));
            }
            GalleryMessage::SetWorkspaceCorners(enabled) => {
                self.appearance.set_workspace_corners_enabled(enabled);
            }
            GalleryMessage::ResetAppearance => {
                self.appearance.reset();
            }
            GalleryMessage::SelectSection(section) => {
                self.section = section;
                self.settings_open = false;
                self.set_workspace_showcase_visible(section == GallerySection::Workspace);
                self.overlay.dismiss();
                self.menu_confirmation.clear();
                self.popover_open = false;
            }
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
                self.overlay.toggle(GalleryOverlay::ContextMenu);
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
                    self.menu_confirmation.clear();
                    self.context_query.clear();
                    self.context_path.clear();
                }
                ContextMenuEvent::Interaction => {}
            },
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

fn gallery_dock() -> DockController {
    let main = DockNode::split(
        DockAxis::Horizontal,
        0.26,
        DockNode::tabs(
            [
                DockId::from("gallery.scenes"),
                DockId::from("gallery.sources"),
            ],
            "gallery.scenes",
        ),
        DockNode::split(
            DockAxis::Vertical,
            0.68,
            DockNode::split(
                DockAxis::Horizontal,
                0.72,
                DockNode::item("gallery.editor"),
                DockNode::tabs(
                    [
                        DockId::from("gallery.properties"),
                        DockId::from("gallery.connection"),
                    ],
                    "gallery.properties",
                ),
            ),
            DockNode::tabs(
                [
                    DockId::from("gallery.mixer"),
                    DockId::from("gallery.cue"),
                    DockId::from("gallery.controls"),
                ],
                "gallery.mixer",
            ),
        ),
    );
    let specs = [
        DockItemSpec::new("gallery.editor", "Studio Editor").limits(360.0, 240.0),
        DockItemSpec::new("gallery.scenes", "Scenes").limits(150.0, 120.0),
        DockItemSpec::new("gallery.sources", "Sources").limits(150.0, 120.0),
        DockItemSpec::new("gallery.properties", "Properties").limits(180.0, 140.0),
        DockItemSpec::new("gallery.connection", "NanaLive").limits(180.0, 140.0),
        DockItemSpec::new("gallery.mixer", "Audio Mixer").limits(240.0, 120.0),
        DockItemSpec::new("gallery.cue", "Cue").limits(140.0, 120.0),
        DockItemSpec::new("gallery.controls", "Controls").limits(140.0, 120.0),
    ];
    DockController::new("gallery.editor", specs, DockLayout::new(main))
        .expect("gallery dock definition is valid")
}

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

fn gallery_calendar_model() -> CalendarHeatmapModel {
    nana_ui::build_calendar_heatmap_model(
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
        ContextMenuItem::new(ContextAction::Duplicate, "复制项目")
            .icon(Icon::Add)
            .keywords(["copy", "duplicate"]),
        ContextMenuItem::new(ContextAction::Rename, "重命名项目")
            .icon(Icon::File)
            .keywords(["edit", "name"]),
        ContextMenuItem::new(ContextAction::Remove, "移除项目")
            .icon(Icon::Close)
            .keywords(["delete", "remove"])
            .confirm_label("再次点击确认移除")
            .danger(true),
    ]
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
        AppearanceEvent::Reset => GalleryMessage::ResetAppearance,
    }
}

fn section_label(section: GallerySection) -> &'static str {
    match section {
        GallerySection::Controls => "控件",
        GallerySection::Surfaces => "表面",
        GallerySection::Feedback => "反馈",
        GallerySection::Workspace => "工作区",
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
