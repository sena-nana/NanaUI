use std::fmt;
use std::sync::{Arc, Mutex};

#[cfg(test)]
use nana_ui::WindowChromeEvent;
use nana_ui::runtime::view::{entity_ref, widget, with_refs};
use nana_ui::runtime::{
    AboutSection, Activate, AppearanceSection, Button, DesktopShell, DocumentId, Entity,
    FrameworkError, IconButton, LayoutViewport, LengthSpec, OverlayHost, RuntimeDocument,
    SemanticColorRole, SettingsBack, SettingsCollapsibleCard, SettingsPage, SettingsSidebar,
    SettingsTabSelected, StableNodeId, ToggleChanged,
};
use nana_ui::theme::type_scale;
use nana_ui::{
    AppearanceEvent, ButtonKind, ControlSize, NanaTextShaper, RegionId, WorkspaceAction,
    WorkspaceModel,
};
use nana_ui_platform::InputPayload;

use super::runtime_host::{
    DEFAULT_VIEWPORT, HostStack, RuntimeChrome, RuntimeSceneInput, ScriptedInput,
    apply_title_bar_maximized, apply_workspace_corners, bind_event, event_point, hugging_text,
    runtime_input_event, search_command_button, sidebar_toggle_button, styled_text, take_pending,
    theme_toggle_button,
};
use super::{GalleryMessage, GalleryState, appearance_message, settings_view};

pub type SettingsRuntimeInput = RuntimeSceneInput;

const SETTINGS_DOCUMENT: u64 = 1;

pub(super) struct GallerySettingsRuntime {
    document: RuntimeDocument,
    shell: Entity<DesktopShell>,
    sidebar: Entity<SettingsSidebar>,
    page: Entity<SettingsPage>,
    appearance: Entity<AppearanceSection>,
    about: Entity<AboutSection>,
    workspace_card: Entity<SettingsCollapsibleCard>,
    sidebar_toggle: Entity<IconButton>,
    search_button: Entity<IconButton>,
    theme_button: Entity<IconButton>,
    reset_workspace: Entity<Button>,
    title_leading: Entity<HostStack>,
    title_center: Entity<nana_ui::runtime::Text>,
    title_trailing: Entity<HostStack>,
    last_viewport: LayoutViewport,
    chrome: RuntimeChrome,
    scripted: ScriptedInput,
    pending: Arc<Mutex<Vec<GalleryMessage>>>,
    text: NanaTextShaper,
    #[cfg(test)]
    last_window_chrome_events: Vec<WindowChromeEvent>,
}

impl fmt::Debug for GallerySettingsRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GallerySettingsRuntime")
            .field("shell", &self.shell.stable_id())
            .field("sidebar", &self.sidebar.stable_id())
            .field("page", &self.page.stable_id())
            .field("appearance", &self.appearance.stable_id())
            .field("about", &self.about.stable_id())
            .field("workspace_card", &self.workspace_card.stable_id())
            .field("last_viewport", &self.last_viewport)
            .finish_non_exhaustive()
    }
}

impl GallerySettingsRuntime {
    fn mount(state: &GalleryState) -> Result<Self, FrameworkError> {
        let pending = Arc::new(Mutex::new(Vec::new()));
        let mut document =
            RuntimeDocument::new(DocumentId::new(SETTINGS_DOCUMENT).expect("settings document id"));
        let document_id = document.document();
        let context = document.context_mut();
        let _ = context.set_theme(state.theme);

        let sidebar_collapsed = state
            .settings_workspace
            .layout()
            .region(&RegionId::Resources)
            .is_some_and(nana_ui::RegionState::collapsed_value);
        let active = page_content_index(&state.settings);
        let (
            _,
            (
                shell,
                (sidebar, page, appearance, about, workspace_card),
                (sidebar_toggle, search_button, theme_button, reset_workspace),
                (title_leading, title_center, title_trailing),
            ),
        ) = context.mount_view_root(document_id, || {
            let shell = entity_ref::<DesktopShell>();
            let sidebar = entity_ref::<SettingsSidebar>();
            let page = entity_ref::<SettingsPage>();
            let appearance = entity_ref::<AppearanceSection>();
            let about = entity_ref::<AboutSection>();
            let workspace_card = entity_ref::<SettingsCollapsibleCard>();
            let [sidebar_toggle, search_button, theme_button] =
                std::array::from_fn(|_| entity_ref::<IconButton>());
            let reset_workspace = entity_ref::<Button>();
            let (title_leading, title_trailing) = (entity_ref(), entity_ref());
            let title_center = entity_ref();
            // The page shows one section; the others stay detached until a
            // tab change hands them to it.
            let content = move |index: usize| {
                move |page: SettingsPage, id: StableNodeId| {
                    if index == active {
                        page.content(id)
                    } else {
                        page
                    }
                }
            };
            let appearance_view = widget(
                AppearanceSection::new(state.theme, state.appearance)
                    .platform_hint(nana_ui::hosted_platform_material_support().hint())
                    .available_materials(nana_ui::hosted_window_material_modes())
                    .material_status(state.material_outcome.status_label()),
            )
            .entity_ref(appearance);
            let summary = widget(HostStack::column(2.0)).children((
                widget(styled_text(
                    settings_view::WORKSPACE_SETTINGS_TITLE,
                    SemanticColorRole::Text,
                    13.0,
                    400,
                )),
                widget(styled_text(
                    settings_view::WORKSPACE_SETTINGS_HINT,
                    SemanticColorRole::Muted,
                    type_scale::HINT,
                    400,
                )),
            ));
            let details = widget(HostStack::column(10.0)).children((
                widget(styled_text(
                    settings_view::WORKSPACE_SETTINGS_DETAILS,
                    SemanticColorRole::Muted,
                    12.0,
                    400,
                )),
                widget(workspace_reset_button()).entity_ref(reset_workspace),
            ));
            let card_view = widget(SettingsCollapsibleCard::new(
                state.workspace_settings_expanded,
            ))
            .entity_ref(workspace_card)
            .summary(summary)
            .details(details);
            let about_view = widget(AboutSection::new(settings_view::gallery_about_metadata()))
                .entity_ref(about);
            let view = widget(
                DesktopShell::from_model(state.settings_workspace.model().clone())
                    .title("NanaUI Gallery"),
            )
            .entity_ref(shell)
            .title_leading(
                widget(HostStack::leading_row(0.0))
                    .entity_ref(title_leading)
                    .children(
                        widget(sidebar_toggle_button(sidebar_collapsed)).entity_ref(sidebar_toggle),
                    ),
            )
            .title_center(
                widget(hugging_text(
                    "NanaUI Gallery",
                    SemanticColorRole::Text,
                    13.0,
                    600,
                ))
                .entity_ref(title_center),
            )
            .title_trailing(
                widget(HostStack::row(6.0))
                    .entity_ref(title_trailing)
                    .children((
                        widget(hugging_text(
                            "设置",
                            SemanticColorRole::Muted,
                            type_scale::HINT,
                            type_scale::REGULAR,
                        )),
                        widget(search_command_button()).entity_ref(search_button),
                        widget(theme_toggle_button(state.theme)).entity_ref(theme_button),
                    )),
            )
            .navigation(
                widget(SettingsSidebar::new(
                    state.settings_model.clone(),
                    state.settings.clone(),
                ))
                .entity_ref(sidebar),
            )
            .primary(
                widget(SettingsPage::new(
                    state.settings_model.clone(),
                    state.settings.clone(),
                ))
                .entity_ref(page)
                .slot(appearance_view, content(0))
                .slot(card_view, content(1))
                .slot(about_view, content(2)),
            );
            with_refs(
                view,
                (
                    shell,
                    (sidebar, page, appearance, about, workspace_card),
                    (sidebar_toggle, search_button, theme_button, reset_workspace),
                    (title_leading, title_center, title_trailing),
                ),
            )
        })?;

        bind_event(
            context,
            sidebar,
            Arc::clone(&pending),
            |event: &SettingsBack| {
                let _ = event;
                GalleryMessage::BackFromSettings
            },
        )?;
        bind_event(
            context,
            sidebar,
            Arc::clone(&pending),
            |event: &SettingsTabSelected| GalleryMessage::SelectSettingsTab(event.tab.clone()),
        )?;
        bind_event(
            context,
            appearance,
            Arc::clone(&pending),
            |event: &AppearanceEvent| appearance_message(*event),
        )?;
        bind_event(
            context,
            workspace_card,
            Arc::clone(&pending),
            |event: &ToggleChanged| {
                let _ = event;
                GalleryMessage::ToggleWorkspaceSettingsDetails
            },
        )?;
        bind_event(
            context,
            reset_workspace,
            Arc::clone(&pending),
            |event: &Activate| {
                let _ = event;
                GalleryMessage::ResetWorkspaceLayout
            },
        )?;
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
        let (width, height) = state.settings_viewport_size();
        let last_viewport = LayoutViewport::new(width, height);
        let mut text = NanaTextShaper::default();
        let _ = document.flush(last_viewport, &mut text);

        Ok(Self {
            document,
            shell,
            sidebar,
            page,
            appearance,
            about,
            workspace_card,
            sidebar_toggle,
            search_button,
            theme_button,
            reset_workspace,
            title_leading,
            title_center,
            title_trailing,
            last_viewport,
            chrome: RuntimeChrome::default(),
            scripted: ScriptedInput::default(),
            pending,
            text,
            #[cfg(test)]
            last_window_chrome_events: Vec::new(),
        })
    }

    fn sync(&mut self, state: &GalleryState) {
        let context = self.document.context_mut();
        let _ = context.set_theme(state.theme);
        let content = page_content_id(
            &state.settings,
            self.appearance.stable_id(),
            self.workspace_card.stable_id(),
            self.about.stable_id(),
        );
        let _ = context.update_component(self.sidebar, |sidebar, _| {
            sidebar.model = state.settings_model.clone();
            sidebar.state = state.settings.clone();
        });
        let _ = context.update_component(self.page, |page, _| {
            page.model = state.settings_model.clone();
            page.state = state.settings.clone();
            page.content = Some(content);
        });
        let _ = context.update_component(self.appearance, |section, _| {
            section.theme = state.theme;
            section.appearance = state.appearance;
            section.platform_hint = Some(Arc::from(
                nana_ui::hosted_platform_material_support().hint(),
            ));
            section.available_materials = nana_ui::hosted_window_material_modes();
            section.material_status = Some(Arc::from(state.material_outcome.status_label()));
        });
        let _ = context.update_component(self.about, |section, _| {
            section.metadata = settings_view::gallery_about_metadata();
        });
        let _ = context.update_component(self.workspace_card, |card, _| {
            card.expanded = state.workspace_settings_expanded;
        });
        let _ = context.update_component(self.reset_workspace, |button, _| {
            *button = workspace_reset_button();
        });
        let sidebar_collapsed = state
            .settings_workspace
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
        let _ = context.update_component(self.shell, |shell, _| {
            shell.model = state.settings_workspace.model().clone();
            shell.title_leading = Some(self.title_leading.into());
            shell.title_center = Some(self.title_center.into());
            shell.title_trailing = Some(self.title_trailing.into());
            shell.navigation = Some(self.sidebar.into());
            shell.primary = Some(self.page.into());
        });
        let _ = context.assemble_settings_sidebar(self.sidebar);
        let _ = context.assemble_appearance_section(self.appearance);
        let _ = context.assemble_about_section(self.about);
        let _ = context.assemble_settings_collapsible_card(self.workspace_card);
        let _ = context.assemble_settings_page(self.page);
        let _ = context.assemble_desktop_shell(self.shell);
        apply_workspace_corners(
            context,
            self.shell,
            state.appearance.workspace_corners_enabled(),
        );
        apply_title_bar_maximized(context, self.shell, state.window_chrome.is_maximized());
        self.flush(state.settings_viewport_size());
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
        let mut messages = take_pending(&self.pending);
        if !self.workspace_is_resizing() {
            messages.extend(self.chrome.title_bar_chrome_messages(&self.document, event));
        }
        #[cfg(test)]
        {
            self.last_window_chrome_events = messages
                .iter()
                .filter_map(|message| match message {
                    GalleryMessage::WindowChrome(event) => Some(*event),
                    _ => None,
                })
                .collect();
        }
        messages
    }

    fn dispatch(&mut self, event: InputPayload) -> Vec<GalleryMessage> {
        self.note_pointer(&event);
        self.scripted.route(&mut self.document, event.clone());
        let mut messages = take_pending(&self.pending);
        if !self.workspace_is_resizing() {
            messages.extend(
                self.chrome
                    .title_bar_chrome_messages(&self.document, &event),
            );
        }
        #[cfg(test)]
        {
            self.last_window_chrome_events = messages
                .iter()
                .filter_map(|message| match message {
                    GalleryMessage::WindowChrome(event) => Some(*event),
                    _ => None,
                })
                .collect();
        }
        messages
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
    fn take_window_chrome_events(&mut self) -> Vec<WindowChromeEvent> {
        std::mem::take(&mut self.last_window_chrome_events)
    }
}

impl GalleryState {
    pub(super) fn settings_viewport_size(&self) -> (f32, f32) {
        self.window_size.unwrap_or(DEFAULT_VIEWPORT)
    }

    pub(super) fn refresh_settings_runtime(&mut self) {
        let (width, height) = self.settings_viewport_size();
        if self.settings_workspace.viewport_geometry().logical_size != (width, height) {
            self.settings_workspace
                .update(WorkspaceAction::WindowResized { width, height });
        }
        if self.settings_runtime.is_none() {
            match GallerySettingsRuntime::mount(self) {
                Ok(runtime) => self.settings_runtime = Some(runtime),
                Err(_) => return,
            }
        }
        if let Some(mut runtime) = self.settings_runtime.take() {
            runtime.sync(self);
            self.settings_runtime = Some(runtime);
        }
    }

    pub(super) fn handle_settings_runtime_input(&mut self, input: SettingsRuntimeInput) {
        if self.settings_runtime.is_none() {
            self.refresh_settings_runtime();
        }
        let Some(mut runtime) = self.settings_runtime.take() else {
            return;
        };
        if let SettingsRuntimeInput::PointerMove(point)
        | SettingsRuntimeInput::PointerDown { point, .. }
        | SettingsRuntimeInput::PointerUp { point, .. } = input
        {
            runtime.chrome.last_pointer = point;
        }
        let event = runtime_input_event(&input, runtime.chrome.last_pointer);
        let messages = runtime.dispatch(event);
        self.settings_runtime = Some(runtime);
        let empty = messages.is_empty();
        for message in messages {
            self.update(message);
        }
        if empty
            && self.settings_open
            && let Some(mut runtime) = self.settings_runtime.take()
        {
            runtime.flush(self.settings_viewport_size());
            self.settings_runtime = Some(runtime);
        }
    }

    #[cfg(test)]
    pub(crate) fn settings_runtime_scene_populated(&self) -> bool {
        self.settings_runtime
            .as_ref()
            .is_some_and(GallerySettingsRuntime::scene_populated)
    }

    #[cfg(test)]
    pub(crate) fn take_settings_window_chrome_events(&mut self) -> Vec<WindowChromeEvent> {
        self.settings_runtime
            .as_mut()
            .map(GallerySettingsRuntime::take_window_chrome_events)
            .unwrap_or_default()
    }
}

fn page_content_id(
    settings: &nana_ui::SettingsState,
    appearance: StableNodeId,
    workspace: StableNodeId,
    about: StableNodeId,
) -> StableNodeId {
    [appearance, workspace, about][page_content_index(settings)]
}

/// Which section the page shows: appearance, workspace or about.
fn page_content_index(settings: &nana_ui::SettingsState) -> usize {
    match settings.active_tab().as_str() {
        "workspace" => 1,
        "about" => 2,
        _ => 0,
    }
}

fn workspace_reset_button() -> Button {
    let mut button = Button::new(settings_view::WORKSPACE_SETTINGS_RESET)
        .kind(ButtonKind::Subtle)
        .size(ControlSize::Small);
    std::sync::Arc::make_mut(&mut button.style.layout).width = Some(LengthSpec::Shrink);
    button
}
