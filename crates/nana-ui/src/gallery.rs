use iced::widget::{
    button, column, container, row, scrollable, slider, space, stack, text, text_editor, toggler,
};
use iced::{Alignment, Element, Length, Point, Size, Subscription, Task};

use crate::components::{
    AboutMetadata, AboutSection, AnchoredMenuPlacement, AnchoredMenuPosition, AppearanceEvent,
    AppearanceSection, Button as UiButton, CalendarHeatmap as UiCalendarHeatmap,
    CalendarHeatmapActiveCell, CalendarHeatmapDatum, CalendarHeatmapEvent, CalendarHeatmapModel,
    CalendarHeatmapOptions, Card as UiCard, Checkbox as UiCheckbox,
    ConfirmDialog as UiConfirmDialog, ContextMenuEvent, ContextMenuHost, ContextMenuItem,
    Dropdown as UiDropdown, DropdownEvent, DropdownOption, IconButton as UiIconButton,
    ImageViewer as UiImageViewer, ImageViewerSource, Input as UiInput,
    InteractiveCard as UiInteractiveCard, ListItem as UiListItem, OverlayHost,
    Popover as UiPopover, Progress as UiProgress, RangeField as UiRangeField,
    SearchDropdown as UiSearchDropdown, SearchDropdownOption, SearchDropdownState, SelectionOption,
    SettingsCollapsibleCard, Switch as UiSwitch, Tabs as UiTabs, Textarea as UiTextarea,
    Tooltip as UiTooltip, XYPad as UiXYPad, XYPadEvent, XYPadValue,
};
use crate::dialog::{DialogClosePolicy, DialogCloseTrigger, DialogSize};
use crate::icons::{Icon, icon, status_indicator};
use crate::layout::{
    RegionId, RegionPlacement, RegionRole, RegionScope, RegionState, WorkspaceLayout,
};
use crate::menu::{MenuConfirmation, MenuSelection};
use crate::overlay::ExclusiveOverlay;
use crate::selection::{SelectionMove, SingleSelection};
use crate::settings::{
    AppearanceSettings, SettingsModel, SettingsState, SettingsTab, SettingsTabId, settings_page,
    settings_sidebar as settings_sidebar_view,
};
use crate::shell::{AppTitleBar, DesktopShell, PopupShell, PopupTitleBarFrame, section_heading};
use crate::sidebar::{
    SidebarFooter, SidebarFooterButton, SidebarFrame, SidebarRow, SidebarRowState, SidebarSection,
};
use crate::theme::{Colors, ThemeMode, ThemeTokens, UI_METRICS, ui_font};
use crate::tooltip::TooltipConfig;
use crate::widgets::{
    ButtonKind, CardKind, button_style, canvas_style, panel_style, scrollable_style, slider_style,
    toggler_style, toolbar_style, vertical_scrollbar,
};
use crate::window_chrome::{WindowChromeEvent, WindowChromeState};
use crate::workspace::{WorkspaceAction, WorkspaceController};

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
    calendar_model: CalendarHeatmapModel,
    calendar_active: Option<CalendarHeatmapActiveCell>,
    selected_item: usize,
    selected_surface_card: usize,
    surface_selection: SingleSelection,
    overlay: ExclusiveOverlay<GalleryOverlay>,
    dialog_policy: DialogClosePolicy,
    menu_confirmation: MenuConfirmation<ContextAction>,
    context_action: Option<ContextAction>,
    context_items: Vec<ContextMenuItem<'static, ContextAction>>,
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
            calendar_model: crate::build_calendar_heatmap_model(
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
            ),
            calendar_active: None,
            selected_item: 0,
            selected_surface_card: 0,
            surface_selection: SingleSelection::new(0),
            overlay: ExclusiveOverlay::new(),
            dialog_policy: DialogClosePolicy::default(),
            menu_confirmation: MenuConfirmation::new(),
            context_action: None,
            context_items: vec![
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
            ],
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

    pub fn view(&self) -> Element<'_, GalleryMessage> {
        let tokens = self.theme_tokens();
        let colors = tokens.colors;
        let shell = if self.settings_open {
            DesktopShell::new(
                self.title_bar(tokens),
                &self.settings_workspace,
                self.settings_content(colors),
                GalleryMessage::Workspace,
                tokens,
            )
            .region(RegionId::Resources, self.settings_sidebar())
        } else {
            let mut shell = DesktopShell::new(
                self.title_bar(tokens),
                &self.workspace,
                self.gallery_content(colors),
                GalleryMessage::Workspace,
                tokens,
            )
            .region(RegionId::Resources, self.gallery_sidebar(colors));
            if self.section == GallerySection::Workspace {
                shell = shell
                    .region(RegionId::PrimaryToolbar, self.workspace_toolbar(colors))
                    .inspector(self.workspace_inspector(colors))
                    .bottom(self.workspace_bottom(colors));
            }
            shell
        };
        let base = shell.view();

        if self.overlay.contains(&GalleryOverlay::Dialog) {
            stack![base, self.dialog(colors)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if self.overlay.contains(&GalleryOverlay::ImageViewer) {
            stack![base, self.image_viewer(colors)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            base
        }
    }

    fn active_workspace(&self) -> &WorkspaceController {
        if self.settings_open {
            &self.settings_workspace
        } else {
            &self.workspace
        }
    }

    fn set_workspace_showcase_visible(&mut self, visible: bool) {
        for id in [
            RegionId::PrimaryToolbar,
            RegionId::Inspector,
            RegionId::Diagnostics,
        ] {
            self.workspace
                .update(WorkspaceAction::SetRegionVisible(id, visible));
        }
    }

    fn title_bar(&self, tokens: ThemeTokens) -> Element<'_, GalleryMessage> {
        let colors = tokens.colors;
        let active_workspace = self.active_workspace();
        let sidebar_collapsed = active_workspace
            .layout()
            .region(&RegionId::Resources)
            .is_some_and(RegionState::collapsed_value);
        let sidebar_toggle = button(icon(Icon::Sidebar, 16.0, colors.muted))
            .width(Length::Fixed(UI_METRICS.icon_button_size))
            .height(Length::Fixed(UI_METRICS.icon_button_size))
            .padding(0)
            .on_press(GalleryMessage::Workspace(WorkspaceAction::ToggleRegion(
                RegionId::Resources,
            )))
            .style(button_style(
                tokens,
                if sidebar_collapsed {
                    ButtonKind::Selected
                } else {
                    ButtonKind::Ghost
                },
            ));
        let theme_icon = match self.theme {
            ThemeMode::Dark => Icon::Appearance,
            ThemeMode::Light => Icon::Moon,
        };
        let context = if self.settings_open {
            "设置"
        } else {
            section_label(self.section)
        };
        let trailing = row![
            text(context).size(11).color(colors.muted),
            button(icon(theme_icon, 14.0, colors.accent))
                .on_press(GalleryMessage::ToggleTheme)
                .width(Length::Fixed(UI_METRICS.icon_button_size))
                .height(Length::Fixed(UI_METRICS.icon_button_size))
                .padding(0)
                .style(button_style(tokens, ButtonKind::Text)),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        AppTitleBar::new("NanaUI Gallery", tokens)
            .leading(sidebar_toggle)
            .trailing(trailing)
            .window_chrome(&self.window_chrome, GalleryMessage::WindowChrome)
            .view()
    }

    fn gallery_sidebar(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let tokens = self.theme_tokens();
        let mut section = SidebarSection::new("Gallery").count(4);
        for (target, label, row_icon) in [
            (GallerySection::Controls, "控件", Icon::Settings),
            (GallerySection::Surfaces, "表面", Icon::Folder),
            (GallerySection::Feedback, "反馈", Icon::About),
            (GallerySection::Workspace, "工作区", Icon::Workspace),
        ] {
            section = section.push(
                SidebarRow::new(label)
                    .leading(icon(row_icon, 14.0, colors.muted))
                    .state(if self.section == target {
                        SidebarRowState::Active
                    } else {
                        SidebarRowState::Idle
                    })
                    .on_select(GalleryMessage::SelectSection(target))
                    .view(tokens),
            );
        }
        let footer = SidebarFooter::new()
            .push(
                SidebarFooterButton::new("设置", Icon::Settings)
                    .on_press(GalleryMessage::OpenSettings)
                    .view(tokens),
            )
            .view(colors);
        SidebarFrame::new(section.view(tokens))
            .footer(footer)
            .view(colors)
    }

    fn gallery_content(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        match self.section {
            GallerySection::Controls => self.controls(colors),
            GallerySection::Surfaces => self.surfaces(colors),
            GallerySection::Feedback => self.feedback(colors),
            GallerySection::Workspace => self.workspace_gallery(colors),
        }
    }

    fn settings_sidebar(&self) -> Element<'_, GalleryMessage> {
        settings_sidebar_view(
            &self.settings_model,
            &self.settings,
            GalleryMessage::BackFromSettings,
            GalleryMessage::SelectSettingsTab,
            self.theme_tokens(),
        )
    }

    fn settings_content(&self, _colors: Colors) -> Element<'_, GalleryMessage> {
        let content = match self.settings.active_tab().as_str() {
            "workspace" => self.workspace_settings(),
            "about" => self.about_settings(),
            _ => self.appearance_settings(),
        };
        settings_page(
            &self.settings_model,
            &self.settings,
            content,
            self.theme_tokens(),
        )
    }

    fn appearance_settings(&self) -> Element<'_, GalleryMessage> {
        AppearanceSection::new(self.theme, &self.appearance, appearance_message)
            .view(self.theme_tokens())
    }

    fn workspace_settings(&self) -> Element<'_, GalleryMessage> {
        let tokens = self.theme_tokens();
        SettingsCollapsibleCard::new(
            column![
                text("工作区布局").size(13),
                text("侧边栏宽度与区域可见状态")
                    .size(11)
                    .color(tokens.colors.muted),
            ]
            .spacing(2),
            column![
                text("恢复默认布局会重置当前工作区的区域尺寸与可见状态。")
                    .size(12)
                    .color(tokens.colors.muted),
                button(text("恢复默认").size(12))
                    .height(Length::Fixed(UI_METRICS.compact_control_height))
                    .padding([0.0, UI_METRICS.control_padding_x])
                    .on_press(GalleryMessage::ResetWorkspaceLayout)
                    .style(button_style(tokens, ButtonKind::Subtle)),
            ]
            .spacing(10),
            self.workspace_settings_expanded,
            GalleryMessage::ToggleWorkspaceSettingsDetails,
        )
        .view(tokens)
    }

    fn about_settings(&self) -> Element<'_, GalleryMessage> {
        AboutSection::new(
            AboutMetadata::new("NanaUI Component Gallery", env!("CARGO_PKG_VERSION"))
                .description("Rust 原生 UI 组件库与工作区框架"),
        )
        .view(self.theme_tokens())
    }

    fn workspace_gallery(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let tokens = self.theme_tokens();
        let toggle = |label: &'static str, id: RegionId| {
            let expanded = region_expanded(self.workspace.layout(), &id);
            button(text(format!("{}{label}", if expanded { "隐藏" } else { "显示" })).size(12))
                .height(Length::Fixed(UI_METRICS.control_height))
                .padding([0.0, UI_METRICS.control_padding_x])
                .on_press(GalleryMessage::Workspace(WorkspaceAction::ToggleRegion(id)))
                .style(button_style(tokens, ButtonKind::Subtle))
        };
        let popup_title_bar = PopupTitleBarFrame::new(
            text("弹窗标题").size(12),
            GalleryMessage::PrimaryAction,
            GalleryMessage::PrimaryAction,
            GalleryMessage::WindowChrome,
            tokens,
        )
        .view();
        let popup = PopupShell::new(
            container(
                column![
                    text("独立弹窗内容").size(13),
                    text("快速创建并管理项目").size(11).color(colors.muted),
                ]
                .spacing(4),
            )
            .center(Length::Fill),
        )
        .title_bar(popup_title_bar)
        .view(tokens);
        container(
            column![
                text("工作区").size(16).color(colors.text),
                container(
                    column![
                        row![
                            toggle("侧栏", RegionId::Resources),
                            toggle("检查器", RegionId::Inspector),
                            toggle("底部面板", RegionId::Diagnostics),
                        ]
                        .spacing(8),
                        text("拖动区域边缘调整尺寸，双击恢复默认值。")
                            .size(12)
                            .color(colors.muted),
                    ]
                    .spacing(10),
                )
                .width(Length::Fill)
                .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
                .style(panel_style(tokens)),
                container(popup)
                    .width(Length::Fixed(360.0))
                    .height(Length::Fixed(150.0))
                    .style(panel_style(tokens)),
            ]
            .spacing(14),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(iced::Padding {
            top: 20.0,
            right: 24.0,
            bottom: 20.0,
            left: 24.0,
        })
        .style(canvas_style(tokens))
        .into()
    }

    fn workspace_toolbar(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let tokens = self.theme_tokens();
        container(
            row![
                text("工作区")
                    .size(13)
                    .font(ui_font(iced::font::Weight::Bold)),
                space().width(Length::Fill),
                button(text("恢复默认").size(12))
                    .height(Length::Fixed(UI_METRICS.compact_control_height))
                    .padding([0.0, UI_METRICS.control_padding_x])
                    .on_press(GalleryMessage::ResetWorkspaceLayout)
                    .style(button_style(tokens, ButtonKind::Text)),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .height(Length::Fill)
        .padding([0, 10])
        .style(toolbar_style(colors))
        .into()
    }

    fn workspace_inspector(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let tokens = self.theme_tokens();
        let radius = self.appearance.standard_radius().round() as u8;
        container(column![
            section_heading::<GalleryMessage>(
                "检查器",
                Some(
                    button(text("收起").size(11))
                        .height(Length::Fixed(UI_METRICS.compact_control_height))
                        .padding([0.0, UI_METRICS.compact_control_padding_x])
                        .on_press(GalleryMessage::Workspace(WorkspaceAction::ToggleRegion(
                            RegionId::Inspector
                        ),))
                        .style(button_style(tokens, ButtonKind::Text))
                        .into(),
                ),
                colors,
            ),
            container(
                column![
                    text("标准圆角").size(11).color(colors.muted),
                    slider(
                        AppearanceSettings::MIN_STANDARD_RADIUS
                            ..=AppearanceSettings::MAX_STANDARD_RADIUS,
                        radius,
                        GalleryMessage::SetStandardRadius,
                    )
                    .height(16)
                    .style(slider_style(colors)),
                    text(format!("{radius} px")).size(11).color(colors.accent),
                    toggler(self.appearance.workspace_corners_enabled())
                        .label("主区域圆角")
                        .on_toggle(GalleryMessage::SetWorkspaceCorners)
                        .size(16)
                        .spacing(8)
                        .text_size(13)
                        .style(toggler_style(colors, false)),
                ]
                .spacing(10),
            )
            .padding([0, 12]),
        ])
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn workspace_bottom(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let tokens = self.theme_tokens();
        container(column![
            section_heading::<GalleryMessage>(
                "底部面板",
                Some(
                    button(text("收起").size(11))
                        .height(Length::Fixed(UI_METRICS.compact_control_height))
                        .padding([0.0, UI_METRICS.compact_control_padding_x])
                        .on_press(GalleryMessage::Workspace(WorkspaceAction::ToggleRegion(
                            RegionId::Diagnostics
                        ),))
                        .style(button_style(tokens, ButtonKind::Text))
                        .into(),
                ),
                colors,
            ),
            container(
                row![
                    status_indicator(true, 10.0, colors.success),
                    text("布局就绪").size(11).color(colors.muted),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            )
            .padding([0, 12]),
        ])
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }

    fn controls(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let input_invalid = self.input.trim().is_empty();
        let editor_invalid = self.editor.text().trim().chars().count() < 4;
        let editor_enabled = self.editor_enabled();
        let loading_button = UiButton::label(if self.loading { "处理中" } else { "加载" })
            .kind(ButtonKind::Text)
            .on_press(GalleryMessage::ToggleLoading)
            .loading(self.loading, self.loading_ticks)
            .view(colors);
        let buttons = container(
            column![
                text("操作").size(12).color(colors.muted),
                row![
                    UiButton::label("次要")
                        .on_press(GalleryMessage::PrimaryAction)
                        .kind(ButtonKind::Subtle)
                        .view(colors),
                    UiButton::label("主要")
                        .on_press(GalleryMessage::PrimaryAction)
                        .kind(ButtonKind::Primary)
                        .view(colors),
                    UiButton::label("禁用")
                        .kind(ButtonKind::Subtle)
                        .disabled(true)
                        .view(colors),
                    loading_button,
                    UiIconButton::new("添加", Icon::Add)
                        .on_press(GalleryMessage::PrimaryAction)
                        .view(colors),
                ]
                .spacing(8),
                text(format!("主要操作已触发 {} 次", self.primary_clicks))
                    .size(10)
                    .color(colors.faint),
            ]
            .spacing(8),
        )
        .width(Length::FillPortion(1))
        .height(Length::Fixed(132.0))
        .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
        .style(panel_style(colors));

        let fields = container(
            column![
                text("字段名称 *")
                    .size(13)
                    .font(ui_font(iced::font::Weight::Semibold)),
                UiInput::new("输入字段名称", &self.input)
                    .on_input(GalleryMessage::InputChanged)
                    .invalid(input_invalid)
                    .view(colors),
                text(if input_invalid {
                    "请输入名称"
                } else {
                    "名称可用"
                })
                .size(12)
                .color(if input_invalid {
                    colors.danger
                } else {
                    colors.success
                }),
                UiSearchDropdown::new(
                    &self.search_dropdown,
                    self.search_selection.as_ref(),
                    GalleryMessage::SelectSearchResult,
                )
                .placeholder("搜索选项")
                .view(colors),
            ]
            .spacing(5),
        )
        .width(Length::FillPortion(1))
        .height(Length::Fixed(132.0))
        .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
        .style(panel_style(colors));

        let editor_toggle = UiSwitch::new(self.switched, "允许编辑说明")
            .on_toggle(GalleryMessage::ToggleSwitch)
            .disabled(!self.checked)
            .view(colors);
        let toggles = container(
            column![
                text("选择控件").size(12).color(colors.muted),
                UiCheckbox::new(self.checked, "启用选项")
                    .on_toggle(GalleryMessage::ToggleCheck)
                    .view(colors),
                editor_toggle,
                row![
                    container(
                        UiRangeField::new(0.0..=100.0, f32::from(self.slider), |value| {
                            GalleryMessage::SetSlider(value.round() as u8)
                        },)
                        .label("强度")
                        .unit("%")
                        .view(colors),
                    )
                    .width(Length::Fill),
                    container(
                        UiDropdown::multiple(
                            self.dropdown_values.iter().copied(),
                            [
                                DropdownOption::new(0, "关闭"),
                                DropdownOption::new(50, "平衡"),
                                DropdownOption::new(100, "最大"),
                            ],
                            GalleryMessage::SetDropdown,
                        )
                        .placeholder("预设")
                        .width(Length::Fill)
                        .view(colors),
                    )
                    .width(Length::Fixed(96.0)),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ]
            .spacing(8),
        )
        .width(Length::FillPortion(1))
        .height(Length::Fixed(132.0))
        .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
        .style(panel_style(colors));

        let editor = UiTextarea::new(&self.editor)
            .placeholder("输入说明")
            .height(96.0)
            .invalid(editor_invalid)
            .disabled(!editor_enabled)
            .on_action(GalleryMessage::EditText)
            .view(colors);
        let text_area = container(
            column![
                text("多行文本")
                    .size(13)
                    .font(ui_font(iced::font::Weight::Semibold)),
                editor,
                text(if editor_invalid {
                    "请至少输入 4 个字符"
                } else if editor_enabled {
                    "说明可编辑"
                } else if !self.checked {
                    "选项停用时不可编辑"
                } else {
                    "说明已锁定"
                })
                .size(12)
                .color(if editor_invalid {
                    colors.danger
                } else {
                    colors.muted
                }),
            ]
            .spacing(5),
        )
        .width(Length::FillPortion(1))
        .height(Length::Fill)
        .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
        .style(panel_style(colors));

        let xy_pad = container(
            column![
                text("二维参数").size(12).color(colors.muted),
                UiXYPad::new(self.xy_pad, GalleryMessage::SetXYPad, colors)
                    .step(0.01)
                    .view(),
                text(format!("X {:.2} · Y {:.2}", self.xy_pad.x, self.xy_pad.y))
                    .size(11)
                    .color(colors.muted),
            ]
            .spacing(8),
        )
        .width(Length::FillPortion(1))
        .height(Length::Fill)
        .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
        .style(panel_style(colors));

        let items = [
            ("默认列表项", false),
            ("选中列表项", false),
            ("带图标列表项", false),
            ("带辅助信息", false),
            ("紧凑列表项", false),
            ("长文本列表项", false),
            ("可操作列表项", false),
            ("禁用列表项", true),
            ("普通状态", false),
            ("悬停状态", false),
            ("按下状态", false),
            ("成功状态", false),
            ("警告状态", false),
            ("错误状态", false),
            ("加载状态", false),
            ("空状态", false),
        ];
        let mut list = column![].spacing(4);
        for (index, (label, disabled)) in items.into_iter().enumerate() {
            let selected = self.selected_item == index;
            let item = UiListItem::label(label)
                .leading(status_indicator(
                    selected,
                    10.0,
                    if selected {
                        colors.accent
                    } else {
                        colors.faint
                    },
                ))
                .trailing(
                    text(if disabled { "不可用" } else { "" })
                        .size(11)
                        .color(colors.muted),
                )
                .selected(selected)
                .disabled(disabled)
                .on_select(GalleryMessage::SelectListItem(index))
                .view(colors);
            list = list.push(item);
        }
        let list = container(
            column![
                text("列表").size(12).color(colors.muted),
                scrollable(list)
                    .direction(vertical_scrollbar())
                    .style(scrollable_style(colors))
                    .height(Length::Fill),
            ]
            .spacing(8),
        )
        .width(Length::FillPortion(1))
        .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
        .height(Length::Fill)
        .style(panel_style(colors));

        container(
            column![
                row![buttons, fields, toggles].spacing(10),
                row![text_area, xy_pad, list].spacing(10)
            ]
            .spacing(10),
        )
        .padding(iced::Padding {
            top: 16.0,
            right: 16.0,
            bottom: 16.0,
            left: 16.0,
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .style(canvas_style(colors))
        .into()
    }

    fn surfaces(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let selected_view = SurfaceView::from_index(self.surface_selection.selected());
        let card = |title: &'static str, detail: &'static str, kind| {
            container(
                UiCard::new(
                    column![
                        text(title).size(13).color(colors.text),
                        text(detail).size(11).color(colors.muted),
                    ]
                    .spacing(6),
                )
                .kind(kind)
                .view(colors),
            )
            .width(Length::FillPortion(1))
            .height(Length::Fixed(96.0))
        };
        let surface_content = match selected_view {
            SurfaceView::Overview => row![
                card("基础表面", "主工作区内容层", CardKind::Surface),
                card("抬升表面", "侧栏与工具面板", CardKind::Raised),
                card("选中表面", "当前激活的内容", CardKind::Selected),
            ],
            SurfaceView::Cards => {
                let cards_data = [
                    ("默认卡片", "普通内容容器", false),
                    ("交互卡片", "支持选择操作", false),
                    ("禁用卡片", "不可进行操作", true),
                ];
                let mut cards = row![].spacing(10);
                for (index, (title, detail, disabled)) in cards_data.into_iter().enumerate() {
                    let selected = self.selected_surface_card == index;
                    let node = UiInteractiveCard::new(
                        column![
                            text(title).size(13),
                            text(detail).size(11).color(colors.muted),
                        ]
                        .spacing(6),
                    )
                    .selected(selected)
                    .disabled(disabled)
                    .on_select(GalleryMessage::SelectSurfaceCard(index))
                    .view(colors);
                    cards = cards.push(
                        container(node)
                            .width(Length::FillPortion(1))
                            .height(Length::Fixed(96.0)),
                    );
                }
                cards
            }
        }
        .spacing(10);
        let segmented = UiTabs::new(
            selected_view,
            [
                SelectionOption::new(SurfaceView::Overview, "概览"),
                SelectionOption::new(SurfaceView::Cards, "卡片"),
            ],
            GalleryMessage::SelectSurfaceView,
        )
        .view(colors);

        container(
            column![
                text("表面层级").size(14).color(colors.text),
                text("基础、抬升与选中状态").size(11).color(colors.muted),
                container(
                    row![
                        text("表面状态").size(12),
                        space().width(Length::Fill),
                        segmented,
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x,])
                .style(panel_style(colors)),
                surface_content,
            ]
            .spacing(12),
        )
        .padding(iced::Padding {
            top: 16.0,
            right: 16.0,
            bottom: 16.0,
            left: 16.0,
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .style(canvas_style(colors))
        .into()
    }

    fn feedback(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let progress = if self.loading { 72.0 } else { 0.0 };
        let tooltip_config = TooltipConfig::default();
        let action_status = match self.context_action {
            Some(ContextAction::Duplicate) => "已复制".to_owned(),
            Some(ContextAction::Rename) => "已重命名".to_owned(),
            Some(ContextAction::Remove) => "已移除".to_owned(),
            None if self.confirmed_actions > 0 => {
                format!("操作已确认 {} 次", self.confirmed_actions)
            }
            None => "等待操作".to_owned(),
        };
        let actions = container(
            column![
                button(
                    text(if self.overlay.contains(&GalleryOverlay::Dialog) {
                        "关闭对话框"
                    } else {
                        "打开对话框"
                    })
                    .size(11),
                )
                .width(Length::Fill)
                .height(Length::Fixed(UI_METRICS.control_height))
                .padding([0.0, UI_METRICS.control_padding_x])
                .on_press(GalleryMessage::ToggleDialog)
                .style(button_style(colors, ButtonKind::Primary)),
                button(text("更多操作").size(11))
                    .width(Length::Fill)
                    .height(Length::Fixed(UI_METRICS.control_height))
                    .padding([0.0, UI_METRICS.control_padding_x])
                    .on_press(GalleryMessage::ToggleContextMenu)
                    .style(button_style(colors, ButtonKind::Subtle)),
                button(text("查看图片").size(11))
                    .width(Length::Fill)
                    .height(Length::Fixed(UI_METRICS.control_height))
                    .padding([0.0, UI_METRICS.control_padding_x])
                    .on_press(GalleryMessage::ToggleImageViewer)
                    .style(button_style(colors, ButtonKind::Subtle)),
                UiPopover::new(
                    UiTooltip::new(
                        container(icon(Icon::About, 13.0, colors.muted))
                            .width(Length::Fixed(UI_METRICS.icon_button_size))
                            .height(Length::Fixed(UI_METRICS.icon_button_size))
                            .align_x(iced::alignment::Horizontal::Center)
                            .align_y(iced::alignment::Vertical::Center),
                        container(text("查看当前状态").size(11)).padding([4, 7]),
                    )
                    .config(tooltip_config)
                    .view(colors),
                    column![
                        text(format!("当前状态：{action_status}")).size(12),
                        UiButton::label("执行主要操作")
                            .kind(ButtonKind::Primary)
                            .on_press(GalleryMessage::PrimaryAction)
                            .view(colors),
                    ]
                    .spacing(8),
                    self.popover_open,
                    GalleryMessage::TogglePopover,
                    GalleryMessage::ClosePopover,
                    colors,
                )
                .view(),
            ]
            .spacing(8)
            .align_x(iced::alignment::Horizontal::Center),
        )
        .width(Length::Fixed(140.0))
        .height(Length::Fixed(160.0))
        .padding([8, 10])
        .style(panel_style(colors));

        let content = container(
            column![
                text("反馈").size(14).color(colors.text),
                row![
                    container(
                        column![
                            text(if self.loading {
                                "处理中"
                            } else {
                                "已完成"
                            })
                            .size(13),
                            UiProgress::<GalleryMessage>::new(progress, 100.0).view(colors),
                        ]
                        .spacing(8),
                    )
                    .width(Length::Fill)
                    .height(Length::Fixed(160.0))
                    .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x,])
                    .style(panel_style(colors)),
                    actions,
                ]
                .spacing(10)
                .align_y(Alignment::Start),
                container(
                    column![
                        text("日历热力图").size(12).color(colors.muted),
                        UiCalendarHeatmap::new(
                            &self.calendar_model,
                            GalleryMessage::CalendarHeatmap,
                            colors,
                        )
                        .view(),
                        text(
                            self.calendar_active
                                .as_ref()
                                .map_or("移动指针查看日期".to_owned(), |cell| cell
                                    .title
                                    .clone()),
                        )
                        .size(10)
                        .color(colors.muted),
                    ]
                    .spacing(6),
                )
                .width(Length::Fill)
                .padding([UI_METRICS.panel_padding_y, UI_METRICS.panel_padding_x])
                .style(panel_style(colors)),
                text(action_status).size(10).color(colors.muted),
            ]
            .spacing(12),
        )
        .padding(iced::Padding {
            top: 16.0,
            right: 16.0,
            bottom: 16.0,
            left: 16.0,
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .style(canvas_style(colors));

        if self.overlay.contains(&GalleryOverlay::ContextMenu) {
            OverlayHost::new(content)
                .push(self.context_menu(colors))
                .view()
        } else {
            content.into()
        }
    }

    fn context_menu(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let (width, height) = self.active_workspace().viewport_geometry().logical_size;
        ContextMenuHost::new(
            &self.context_items,
            AnchoredMenuPosition::new(Point::new(width - 24.0, 112.0))
                .placement(AnchoredMenuPlacement::BottomEnd),
            Size::new(width, height),
            GalleryMessage::ContextMenu,
            colors,
        )
        .search(&self.context_query, true)
        .active_path(&self.context_path)
        .pending(self.menu_confirmation.pending())
        .view()
    }

    fn dialog(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        UiConfirmDialog::new(
            "确认操作",
            "确认后将记录一次完整操作。",
            GalleryMessage::ConfirmDialog,
            GalleryMessage::RequestDialogClose(DialogCloseTrigger::CloseButton),
            GalleryMessage::OverlayInteraction,
        )
        .description("此操作会更新当前状态")
        .on_outside(GalleryMessage::RequestDialogClose(
            DialogCloseTrigger::Outside,
        ))
        .size(DialogSize::Default)
        .view(colors)
    }

    fn image_viewer(&self, colors: Colors) -> Element<'_, GalleryMessage> {
        let preview = container(
            stack![
                container(space())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(move |_theme| {
                        iced::widget::container::Style::default().background(colors.accent_strong)
                    }),
                container(
                    column![
                        text("NANA").size(48).color(colors.accent_text),
                        text("完整组件库").size(14).color(colors.accent_text),
                    ]
                    .spacing(6)
                    .align_x(iced::alignment::Horizontal::Center),
                )
                .center(Length::Fill),
            ]
            .width(Length::Fill)
            .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill);
        UiImageViewer::new(
            ImageViewerSource::new(preview)
                .name("NanaUI 渲染预览")
                .metadata("预览图 · 1600 × 900"),
            GalleryMessage::RequestImageViewerClose(DialogCloseTrigger::CloseButton),
            GalleryMessage::RequestImageViewerClose(DialogCloseTrigger::Outside),
            GalleryMessage::OverlayInteraction,
            colors,
        )
        .view()
    }
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

fn region_expanded(layout: &WorkspaceLayout, id: &RegionId) -> bool {
    layout
        .region(id)
        .is_some_and(|region| !region.collapsed_value() && !region.hidden_value())
}

#[cfg(test)]
mod tests {
    use super::{
        ContextAction, ContextMenuEvent, GalleryMessage, GalleryOverlay, GallerySection,
        GalleryState, SurfaceView,
    };
    use crate::layout::RegionId;
    use crate::selection::SelectionMove;
    use crate::theme::ThemeMode;
    use crate::workspace::WorkspaceAction;

    #[test]
    fn gallery_interactions_update_real_state() {
        let mut state = GalleryState::new();
        state.update(GalleryMessage::PrimaryAction);
        state.update(GalleryMessage::ToggleLoading);
        state.update(GalleryMessage::InputChanged("Field".to_owned()));
        state.update(GalleryMessage::SelectSection(GallerySection::Feedback));
        state.update(GalleryMessage::ToggleContextMenu);
        state.update(GalleryMessage::ContextMenu(ContextMenuEvent::Select(
            ContextAction::Rename,
        )));

        assert_eq!(state.primary_clicks, 1);
        assert!(state.loading);
        assert_eq!(state.input, "Field");
        assert_eq!(state.section, GallerySection::Feedback);
        assert!(!state.overlay.is_open());
        assert_eq!(state.context_action, Some(ContextAction::Rename));
    }

    #[test]
    fn gallery_overlays_are_mutually_exclusive_and_dismissible() {
        let mut state = GalleryState::new();
        state.update(GalleryMessage::ToggleDialog);
        assert!(state.overlay.contains(&GalleryOverlay::Dialog));

        state.update(GalleryMessage::ToggleContextMenu);
        assert!(state.overlay.contains(&GalleryOverlay::ContextMenu));
        assert!(!state.overlay.contains(&GalleryOverlay::Dialog));

        state.update(GalleryMessage::DismissOverlay);
        assert!(!state.overlay.is_open());
    }

    #[test]
    fn destructive_menu_action_requires_confirmation() {
        let mut state = GalleryState::new();
        state.update(GalleryMessage::SelectListItem(2));
        state.update(GalleryMessage::SelectSection(GallerySection::Feedback));
        state.update(GalleryMessage::ToggleContextMenu);

        state.update(GalleryMessage::ContextMenu(ContextMenuEvent::Select(
            ContextAction::Remove,
        )));
        assert!(state.overlay.contains(&GalleryOverlay::ContextMenu));
        assert_eq!(
            state.menu_confirmation.pending(),
            Some(&ContextAction::Remove)
        );
        assert_eq!(state.selected_item, 2);

        state.update(GalleryMessage::ContextMenu(ContextMenuEvent::Select(
            ContextAction::Remove,
        )));
        assert!(!state.overlay.is_open());
        assert_eq!(state.context_action, Some(ContextAction::Remove));
        assert_eq!(state.selected_item, 0);
    }

    #[test]
    fn dialog_confirmation_executes_and_closes_the_overlay() {
        let mut state = GalleryState::new();
        state.update(GalleryMessage::ToggleDialog);
        assert!(state.overlay.contains(&GalleryOverlay::Dialog));

        state.update(GalleryMessage::ConfirmDialog);
        assert!(!state.overlay.is_open());
        assert_eq!(state.confirmed_actions, 1);
    }

    #[test]
    fn segmented_surface_view_supports_click_and_roving_selection() {
        let mut state = GalleryState::new();
        state.update(GalleryMessage::SelectSurfaceView(SurfaceView::Cards));
        assert_eq!(state.surface_selection.selected(), 1);

        state.update(GalleryMessage::SelectSurfaceCard(1));
        assert_eq!(state.selected_surface_card, 1);

        state.update(GalleryMessage::NavigateSurfaceView(SelectionMove::Next));
        assert_eq!(state.surface_selection.selected(), 0);
        state.update(GalleryMessage::NavigateSurfaceView(SelectionMove::Last));
        assert_eq!(state.surface_selection.selected(), 1);
    }

    #[test]
    fn loading_state_blocks_until_the_async_cycle_finishes() {
        let mut state = GalleryState::new();
        state.update(GalleryMessage::ToggleLoading);
        assert!(state.loading);

        for _ in 0..11 {
            state.update(GalleryMessage::LoadingTick);
            assert!(state.loading);
        }
        state.update(GalleryMessage::LoadingTick);
        assert!(!state.loading);
        assert_eq!(state.loading_ticks, 0);
    }

    #[test]
    fn selection_and_edit_switches_control_editor_availability() {
        let mut state = GalleryState::new();
        assert!(state.editor_enabled());

        state.update(GalleryMessage::ToggleCheck(false));
        assert!(!state.editor_enabled());

        state.update(GalleryMessage::ToggleCheck(true));
        state.update(GalleryMessage::ToggleSwitch(false));
        assert!(!state.editor_enabled());
    }

    #[test]
    fn workspace_section_controls_auxiliary_regions_without_losing_sizes() {
        let mut state = GalleryState::new();
        for id in [
            RegionId::PrimaryToolbar,
            RegionId::Inspector,
            RegionId::Diagnostics,
        ] {
            assert!(
                state
                    .workspace
                    .layout()
                    .region(&id)
                    .expect("gallery region")
                    .hidden_value()
            );
        }

        state.update(GalleryMessage::SelectSection(GallerySection::Workspace));
        state.update(GalleryMessage::Workspace(WorkspaceAction::SetRegionSize(
            RegionId::Inspector,
            340.0,
        )));
        assert!(
            !state
                .workspace
                .layout()
                .region(&RegionId::Inspector)
                .expect("inspector")
                .hidden_value()
        );

        state.update(GalleryMessage::SelectSection(GallerySection::Controls));
        assert!(
            state
                .workspace
                .layout()
                .region(&RegionId::Inspector)
                .expect("inspector")
                .hidden_value()
        );

        state.update(GalleryMessage::SelectSection(GallerySection::Workspace));
        let inspector = state
            .workspace
            .layout()
            .region(&RegionId::Inspector)
            .expect("inspector");
        assert!(!inspector.hidden_value());
        assert_eq!(inspector.size_value(), Some(340.0));
    }

    #[test]
    fn settings_return_to_the_gallery_and_appearance_updates_immediately() {
        let mut state = GalleryState::new();
        state.update(GalleryMessage::SelectSection(GallerySection::Surfaces));
        state.update(GalleryMessage::OpenSettings);
        assert!(state.settings_open);

        state.update(GalleryMessage::SetTheme(ThemeMode::Light));
        state.update(GalleryMessage::SetStandardRadius(8));
        state.update(GalleryMessage::BackFromSettings);

        assert!(!state.settings_open);
        assert_eq!(state.section, GallerySection::Surfaces);
        assert_eq!(state.theme_mode(), ThemeMode::Light);
        assert_eq!(state.appearance.standard_radius(), 8.0);
    }
}
