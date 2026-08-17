use std::fmt;
use std::sync::{Arc, Mutex};

use nana_ui::runtime::{
    AlignSpec, AppContext, Button, CommandPalette, ConfirmDialog, ConfirmIntent, ConfirmSlots,
    ContextMenu, ContextMenuEvent as RuntimeContextMenuEvent,
    ContextMenuItem as RuntimeContextMenuItem, DesktopShell, DocumentId, Entity, FrameworkError,
    IconButton, ImageViewer, ImageViewerContent, ImageViewerEvent, LayoutBox, OverlayChanged,
    OverlayHost, RuntimeDocument, SemanticColorRole, Text,
};
#[cfg(test)]
use nana_ui::runtime::{StableNodeId, StandardVisual};
use nana_ui::{
    ButtonKind, CommandPaletteEvent, ControlSize, Icon, LogicalPoint, RuntimeInputAdapter,
};
use nana_ui_platform::{InputEvent, PointerPhase};

use super::runtime_host::{
    HostStack, RuntimeSceneInput, bind_event, hugging_text, runtime_input_event, styled_text,
    take_pending,
};
use super::{
    ContextAction, DialogCloseTrigger, GalleryContextMenuEvent, GalleryMessage, GalleryOverlay,
    GalleryState,
};

type OverlayTarget = (
    DocumentId,
    Entity<DesktopShell>,
    Entity<OverlayHost>,
    Arc<Mutex<Vec<GalleryMessage>>>,
);

const PALETTE_TITLE: &str = "命令";
const PALETTE_PLACEHOLDER: &str = "搜索命令";
const DIALOG_TITLE: &str = "确认操作";
const DIALOG_DESCRIPTION: &str = "此操作会更新当前状态";
const DIALOG_MESSAGE: &str = "确认后将记录一次完整操作。";
const IMAGE_PREVIEW_NAME: &str = "NanaUI 渲染预览";
const IMAGE_PREVIEW_METADATA: &str = "预览图 · 1600 × 900";
const IMAGE_PREVIEW_TITLE: &str = "NANA";
const IMAGE_PREVIEW_CAPTION: &str = "完整组件库";
const IMAGE_PREVIEW_INSET: f32 = 54.0;
const CONTEXT_GROUP: &str = "project";

pub(super) struct GalleryOverlaysRuntime {
    kind: GalleryOverlay,
    document_id: DocumentId,
    shell: Entity<DesktopShell>,
    host: Entity<OverlayHost>,
    palette: Option<Entity<CommandPalette>>,
    #[allow(dead_code)]
    dialog: Option<Entity<ConfirmDialog>>,
    image: Option<Entity<ImageViewer>>,
    menu: Option<Entity<ContextMenu>>,
    last_menu_path: Vec<usize>,
    last_pointer: LogicalPoint,
}

impl fmt::Debug for GalleryOverlaysRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GalleryOverlaysRuntime")
            .field("kind", &self.kind)
            .field("document_id", &self.document_id)
            .finish_non_exhaustive()
    }
}

impl GalleryOverlaysRuntime {
    fn mount(
        document: &mut RuntimeDocument,
        shell: Entity<DesktopShell>,
        host: Entity<OverlayHost>,
        state: &GalleryState,
        kind: GalleryOverlay,
        pending: &Arc<Mutex<Vec<GalleryMessage>>>,
        bind_host: bool,
    ) -> Result<Self, FrameworkError> {
        let document_id = document.document();
        let context = document.context_mut();
        let _ = context.set_theme(state.theme);
        let mut palette = None;
        let mut dialog = None;
        let mut image = None;
        let mut menu = None;
        let overlay_id = match kind {
            GalleryOverlay::CommandPalette => {
                let overlay = context
                    .create_detached_component(document_id, gallery_command_palette(state))?;
                bind_event(
                    context,
                    overlay,
                    Arc::clone(pending),
                    |event: &CommandPaletteEvent| match event {
                        CommandPaletteEvent::Navigate(_) => GalleryMessage::OverlayInteraction,
                        event => GalleryMessage::CommandPalette(event.clone()),
                    },
                )?;
                context.append_child(host, overlay)?;
                context.activate_overlay(host, overlay)?;
                palette = Some(overlay);
                overlay.stable_id()
            }
            GalleryOverlay::Dialog => {
                let overlay = context.create_detached_component(
                    document_id,
                    ConfirmDialog::new(DIALOG_TITLE, DIALOG_DESCRIPTION),
                )?;
                let body = context.create_detached_component(document_id, dialog_body_text())?;
                let close = context.create_detached_component(
                    document_id,
                    IconButton::new(Icon::Close, "关闭")
                        .size(ControlSize::Small)
                        .kind(ButtonKind::Text),
                )?;
                let cancel = context.create_detached_component(
                    document_id,
                    Button::new("取消").kind(ButtonKind::Ghost),
                )?;
                let confirm = context.create_detached_component(
                    document_id,
                    Button::new("确认").kind(ButtonKind::Primary),
                )?;
                context.set_confirm_slots(
                    overlay,
                    ConfirmSlots {
                        body: Some(body.stable_id()),
                        close_action: Some(close.stable_id()),
                        cancel: cancel.stable_id(),
                        secondary: None,
                        confirm: confirm.stable_id(),
                    },
                )?;
                bind_event(
                    context,
                    overlay,
                    Arc::clone(pending),
                    |intent: &ConfirmIntent| match intent {
                        ConfirmIntent::Confirm { .. } => GalleryMessage::ConfirmDialog,
                        ConfirmIntent::Cancel | ConfirmIntent::Secondary => {
                            GalleryMessage::RequestDialogClose(DialogCloseTrigger::CloseButton)
                        }
                    },
                )?;
                context.append_child(host, overlay)?;
                context.activate_overlay(host, overlay)?;
                dialog = Some(overlay);
                overlay.stable_id()
            }
            GalleryOverlay::ImageViewer => {
                let preview = mount_image_preview(context, document_id)?;
                let overlay = context.create_detached_component(
                    document_id,
                    ImageViewer::new(ImageViewerContent::child(preview.stable_id()))
                        .name(IMAGE_PREVIEW_NAME)
                        .metadata(IMAGE_PREVIEW_METADATA),
                )?;
                context.append_child(overlay, preview)?;
                bind_event(
                    context,
                    overlay,
                    Arc::clone(pending),
                    |event: &ImageViewerEvent| match event {
                        ImageViewerEvent::Close => {
                            GalleryMessage::RequestImageViewerClose(DialogCloseTrigger::CloseButton)
                        }
                        ImageViewerEvent::Outside => {
                            GalleryMessage::RequestImageViewerClose(DialogCloseTrigger::Outside)
                        }
                        ImageViewerEvent::Interaction => GalleryMessage::OverlayInteraction,
                    },
                )?;
                context.append_child(host, overlay)?;
                context.activate_overlay(host, overlay)?;
                image = Some(overlay);
                overlay.stable_id()
            }
            GalleryOverlay::ContextMenu => {
                let (anchor_x, anchor_y) = context_menu_anchor(state);
                let overlay = context.create_detached_component(
                    document_id,
                    ContextMenu::new(anchor_x, anchor_y)
                        .items(runtime_context_items(state.context_items()))
                        .query(state.context_query.as_str())
                        .searchable(true)
                        .active_path(runtime_menu_path(&state.context_path))
                        .open(true),
                )?;
                bind_event(
                    context,
                    overlay,
                    Arc::clone(pending),
                    |event: &RuntimeContextMenuEvent| match event {
                        RuntimeContextMenuEvent::Search(query) => GalleryMessage::ContextMenu(
                            GalleryContextMenuEvent::Search(query.to_string()),
                        ),
                        RuntimeContextMenuEvent::Select(value) => {
                            match context_action_from_value(value) {
                                Some(action) => {
                                    GalleryMessage::ContextMenu(GalleryContextMenuEvent::Select(
                                        context_action_key(action).to_string(),
                                    ))
                                }
                                None => GalleryMessage::OverlayInteraction,
                            }
                        }
                        RuntimeContextMenuEvent::Dismiss => {
                            GalleryMessage::ContextMenu(GalleryContextMenuEvent::Dismiss)
                        }
                    },
                )?;
                context.append_child(host, overlay)?;
                context.activate_overlay(host, overlay)?;
                menu = Some(overlay);
                overlay.stable_id()
            }
        };

        let _ = context.update_component(shell, |shell, _| {
            shell.overlays = vec![overlay_id];
        });
        context.assemble_desktop_shell(shell)?;
        if let Some(overlay) = palette {
            context.activate_overlay(host, overlay)?;
        } else if let Some(overlay) = dialog {
            context.activate_overlay(host, overlay)?;
        } else if let Some(overlay) = image {
            context.activate_overlay(host, overlay)?;
        } else if let Some(overlay) = menu {
            context.activate_overlay(host, overlay)?;
        }
        if bind_host {
            bind_event(
                context,
                host,
                Arc::clone(pending),
                |event: &OverlayChanged| {
                    if event.active.is_none() {
                        GalleryMessage::DismissOverlay
                    } else {
                        GalleryMessage::OverlayInteraction
                    }
                },
            )?;
        }

        Ok(Self {
            kind,
            document_id,
            shell,
            host,
            palette,
            dialog,
            image,
            menu,
            last_menu_path: state.context_path.clone(),
            last_pointer: LogicalPoint::default(),
        })
    }

    fn detach(&self, document: &mut RuntimeDocument) {
        let context = document.context_mut();
        let _ = context.dismiss_overlay(self.host);
        let _ = context.update_component(self.shell, |shell, _| {
            shell.overlays.clear();
        });
        let _ = context.assemble_desktop_shell(self.shell);
    }

    fn sync(&mut self, document: &mut RuntimeDocument, state: &GalleryState) {
        let context = document.context_mut();
        let _ = context.set_theme(state.theme);
        match self.kind {
            GalleryOverlay::CommandPalette => {
                if let Some(palette) = self.palette {
                    let items = state.palette_items();
                    let query = state.action_picker.query().to_owned();
                    let selected = state.action_picker.selected();
                    let _ = context.update_component(palette, |palette, _| {
                        palette.title = Arc::from(PALETTE_TITLE);
                        palette.placeholder = Arc::from(PALETTE_PLACEHOLDER);
                        palette.items = items;
                        let _ = palette.set_query(query);
                        palette.selected = selected;
                    });
                }
            }
            GalleryOverlay::ContextMenu => {
                if let Some(menu) = self.menu {
                    let (anchor_x, anchor_y) = context_menu_anchor(state);
                    let items = runtime_context_items(state.context_items());
                    let query = state.context_query.clone();
                    let path = runtime_menu_path(&state.context_path);
                    let (width, height) = state.gallery_viewport_size();
                    let _ = context.update_component(menu, |menu, _| {
                        menu.items = items;
                        menu.anchor_x = anchor_x;
                        menu.anchor_y = anchor_y;
                        menu.set_query(query);
                        menu.active_path = path;
                        menu.searchable = true;
                        menu.open = true;
                        menu.place_in(LayoutBox {
                            x: 0.0,
                            y: 0.0,
                            width,
                            height,
                        });
                    });
                    self.last_menu_path = state.context_path.clone();
                }
            }
            GalleryOverlay::Dialog | GalleryOverlay::ImageViewer => {}
        }
    }

    fn apply_pointer(&mut self, document: &mut RuntimeDocument, event: &InputEvent) {
        if let InputEvent::Pointer { x, y, .. } | InputEvent::Wheel { x, y, .. } = *event {
            self.last_pointer = LogicalPoint::new(x, y);
        }
        let Some(viewer) = self.image else {
            return;
        };
        let context = document.context_mut();
        match *event {
            InputEvent::Pointer {
                phase: PointerPhase::Down,
                pointer_id,
                x,
                y,
                ..
            } => {
                let _ = context.image_viewer_pointer_down(viewer, pointer_id, x, y);
            }
            InputEvent::Pointer {
                phase: PointerPhase::Move,
                pointer_id,
                x,
                y,
                ..
            } => {
                let _ = context.image_viewer_pointer_move(viewer, pointer_id, x, y);
            }
            InputEvent::Pointer {
                phase: PointerPhase::Up | PointerPhase::Cancel,
                pointer_id,
                ..
            } => {
                let _ = context.image_viewer_pointer_up(viewer, pointer_id);
            }
            InputEvent::Wheel { x, y, delta_y, .. } => {
                let _ = context.image_viewer_wheel(viewer, x, y, delta_y);
            }
            _ => {}
        }
    }

    fn take_messages(
        &mut self,
        document: &RuntimeDocument,
        pending: &Arc<Mutex<Vec<GalleryMessage>>>,
    ) -> Vec<GalleryMessage> {
        let mut messages = take_pending(pending);
        if let Some(menu) = self.menu
            && let Ok(path) = document
                .context()
                .read(menu, |menu| menu.active_path.clone())
        {
            let gallery_path = gallery_menu_path(&path);
            if gallery_path != self.last_menu_path {
                self.last_menu_path = gallery_path.clone();
                messages.push(GalleryMessage::ContextMenu(
                    GalleryContextMenuEvent::OpenSubmenu(gallery_path),
                ));
            }
        }
        messages
    }

    fn dispatch(
        &mut self,
        document: &mut RuntimeDocument,
        event: InputEvent,
        pending: &Arc<Mutex<Vec<GalleryMessage>>>,
    ) -> Vec<GalleryMessage> {
        self.apply_pointer(document, &event);
        let document_id = document.document();
        let _ =
            RuntimeInputAdapter::default().dispatch(document.context_mut(), document_id, &event);
        self.take_messages(document, pending)
    }

    fn matches_host(&self, document_id: DocumentId, host: Entity<OverlayHost>) -> bool {
        self.document_id == document_id && self.host == host
    }

    #[cfg(test)]
    fn attached_overlay(&self) -> Option<StableNodeId> {
        self.palette
            .map(Entity::stable_id)
            .or_else(|| self.dialog.map(Entity::stable_id))
            .or_else(|| self.image.map(Entity::stable_id))
            .or_else(|| self.menu.map(Entity::stable_id))
    }
}

impl GalleryState {
    pub(super) fn refresh_overlay_runtime(&mut self) {
        let Some(kind) = self.overlay.active().copied() else {
            self.detach_overlay_runtime();
            return;
        };
        let Some((document_id, shell, host, pending)) = self.active_overlay_target() else {
            self.overlay_runtime = None;
            return;
        };
        let remount = self
            .overlay_runtime
            .as_ref()
            .is_none_or(|runtime| runtime.kind != kind || !runtime.matches_host(document_id, host));
        if remount {
            let bind_host = self
                .overlay_runtime
                .as_ref()
                .is_none_or(|runtime| !runtime.matches_host(document_id, host));
            self.detach_overlay_runtime();
            let mounted = self.with_active_document_mut(|document, state| {
                GalleryOverlaysRuntime::mount(
                    document, shell, host, state, kind, &pending, bind_host,
                )
            });
            match mounted {
                Some(Ok(runtime)) => self.overlay_runtime = Some(runtime),
                _ => {
                    self.overlay_runtime = None;
                    return;
                }
            }
        }
        if let Some(mut runtime) = self.overlay_runtime.take() {
            self.with_active_document_mut(|document, state| runtime.sync(document, state));
            self.overlay_runtime = Some(runtime);
        }
        self.flush_active_document();
    }

    fn detach_overlay_runtime(&mut self) {
        let Some(runtime) = self.overlay_runtime.take() else {
            return;
        };
        self.with_active_document_mut(|document, _| runtime.detach(document));
        self.flush_active_document();
    }

    pub(super) fn handle_overlay_runtime_input(&mut self, input: RuntimeSceneInput) {
        if self.overlay_runtime.is_none() {
            self.refresh_overlay_runtime();
        }
        let Some(mut runtime) = self.overlay_runtime.take() else {
            return;
        };
        if let RuntimeSceneInput::PointerMove(point)
        | RuntimeSceneInput::PointerDown { point, .. }
        | RuntimeSceneInput::PointerUp { point, .. } = input
        {
            runtime.last_pointer = point;
        }
        let event = overlay_input_event(&input, runtime.last_pointer);
        let pending = self.active_runtime_pending();
        let messages = self
            .with_active_document_mut(|document, _| runtime.dispatch(document, event, &pending))
            .unwrap_or_default();
        self.overlay_runtime = Some(runtime);
        self.flush_active_document();
        for message in messages {
            self.update(message);
        }
        if self.overlay.is_open() {
            self.refresh_overlay_runtime();
        } else {
            self.detach_overlay_runtime();
        }
    }

    pub(super) fn apply_overlay_host_input(&mut self, event: &InputEvent) -> Vec<GalleryMessage> {
        let Some(mut runtime) = self.overlay_runtime.take() else {
            return Vec::new();
        };
        let pending = self.active_runtime_pending();
        runtime.apply_pointer_on_active(self, event);
        let messages = self
            .with_active_document(|document| runtime.take_messages(document, &pending))
            .unwrap_or_default();
        self.overlay_runtime = Some(runtime);
        messages
    }

    fn active_overlay_target(&self) -> Option<OverlayTarget> {
        if self.settings_open {
            let runtime = self.settings_runtime.as_ref()?;
            Some((
                runtime.runtime_document().document(),
                runtime.shell(),
                runtime.overlay_host()?,
                runtime.pending_sink(),
            ))
        } else {
            let runtime = self.gallery_runtime.as_ref()?;
            Some((
                runtime.runtime_document().document(),
                runtime.shell(),
                runtime.overlay_host()?,
                runtime.pending_sink(),
            ))
        }
    }

    fn active_runtime_pending(&self) -> Arc<Mutex<Vec<GalleryMessage>>> {
        if self.settings_open {
            self.settings_runtime
                .as_ref()
                .map(super::runtime_settings::GallerySettingsRuntime::pending_sink)
        } else {
            self.gallery_runtime
                .as_ref()
                .map(super::runtime_gallery::GalleryRuntime::pending_sink)
        }
        .unwrap_or_else(|| Arc::new(Mutex::new(Vec::new())))
    }

    pub(super) fn with_active_document<R>(
        &self,
        f: impl FnOnce(&RuntimeDocument) -> R,
    ) -> Option<R> {
        if self.settings_open {
            self.settings_runtime
                .as_ref()
                .map(|runtime| f(runtime.runtime_document()))
        } else {
            self.gallery_runtime
                .as_ref()
                .map(|runtime| f(runtime.runtime_document()))
        }
    }

    fn with_active_document_mut<R>(
        &mut self,
        f: impl FnOnce(&mut RuntimeDocument, &GalleryState) -> R,
    ) -> Option<R> {
        if self.settings_open {
            let mut runtime = self.settings_runtime.take()?;
            let result = f(runtime.runtime_document_mut(), self);
            self.settings_runtime = Some(runtime);
            Some(result)
        } else {
            let mut runtime = self.gallery_runtime.take()?;
            let result = f(runtime.runtime_document_mut(), self);
            self.gallery_runtime = Some(runtime);
            Some(result)
        }
    }

    fn flush_active_document(&mut self) {
        if self.settings_open {
            let size = self.settings_viewport_size();
            if let Some(runtime) = self.settings_runtime.as_mut() {
                runtime.flush_viewport(size);
            }
        } else {
            let size = self.gallery_viewport_size();
            if let Some(runtime) = self.gallery_runtime.as_mut() {
                runtime.flush_viewport(size);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn gallery_overlay_runtime_scene_populated(&self) -> bool {
        let Some(runtime) = self.overlay_runtime.as_ref() else {
            return false;
        };
        let Some(overlay) = runtime.attached_overlay() else {
            return false;
        };
        self.with_active_document(|document| {
            !document.scene().is_empty()
                && document
                    .context()
                    .world()
                    .overlay_host(runtime.host.stable_id())
                    .and_then(|host| host.active)
                    == Some(overlay)
        })
        .unwrap_or(false)
    }

    #[cfg(test)]
    pub(crate) fn gallery_overlay_command_palette_state(
        &self,
    ) -> Option<(String, String, String, usize)> {
        let runtime = self.overlay_runtime.as_ref()?;
        let palette = runtime.palette?;
        self.with_active_document(|document| {
            document.context().read(palette, |palette| {
                (
                    palette.title.to_string(),
                    palette.query.clone(),
                    palette.state.value.clone(),
                    palette.selected,
                )
            })
        })?
        .ok()
    }

    #[cfg(test)]
    pub(crate) fn gallery_overlay_command_palette_visual(&self) -> Option<(String, String)> {
        let runtime = self.overlay_runtime.as_ref()?;
        let palette = runtime.palette?;
        self.with_active_document(|document| {
            match document
                .context()
                .world()
                .standard_visual(palette.stable_id())
            {
                Some(StandardVisual::CommandPalette { title, query, .. }) => {
                    Some((title.to_string(), query.to_string()))
                }
                _ => None,
            }
        })?
    }

    #[cfg(test)]
    pub(crate) fn gallery_overlay_dialog_copy(&self) -> Option<(String, String)> {
        let runtime = self.overlay_runtime.as_ref()?;
        let dialog = runtime.dialog?;
        self.with_active_document(|document| {
            document.context().read(dialog, |dialog| {
                (dialog.title.to_string(), dialog.message.to_string())
            })
        })?
        .ok()
    }

    #[cfg(test)]
    pub(crate) fn gallery_overlay_image_preview(&self) -> Option<(bool, Vec<String>)> {
        let runtime = self.overlay_runtime.as_ref()?;
        let viewer = runtime.image?;
        let content = self
            .with_active_document(|document| {
                document
                    .context()
                    .read(viewer, |viewer| viewer.content.clone())
            })?
            .ok()?;
        let ImageViewerContent::Child(child) = content else {
            return Some((false, Vec::new()));
        };
        Some((
            true,
            self.with_active_document(|document| collect_node_text(document.context(), child))?,
        ))
    }

    #[cfg(test)]
    pub(crate) fn overlay_shares_active_document(&self) -> bool {
        let Some(runtime) = self.overlay_runtime.as_ref() else {
            return false;
        };
        self.with_active_document(|document| document.document() == runtime.document_id)
            .unwrap_or(false)
    }
}

impl GalleryOverlaysRuntime {
    fn apply_pointer_on_active(&mut self, state: &mut GalleryState, event: &InputEvent) {
        state.with_active_document_mut(|document, _| self.apply_pointer(document, event));
    }
}

#[cfg(test)]
pub(crate) fn gallery_context_action_from_value(value: &str) -> Option<ContextAction> {
    context_action_from_value(value)
}

#[cfg(test)]
pub(crate) fn gallery_runtime_context_item_icons(
    items: &[nana_ui::ContextMenuItem],
) -> Vec<(String, String, Option<Icon>)> {
    items
        .iter()
        .map(|item| (item.value.to_string(), item.label.to_string(), item.icon))
        .collect()
}

#[cfg(test)]
fn collect_node_text(context: &AppContext, root: nana_ui::runtime::StableNodeId) -> Vec<String> {
    let mut texts = Vec::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if let Some(text) = context.world().text(id)
            && !text.is_empty()
        {
            texts.push(text.to_owned());
        }
        if let Some(node) = context.world().node(id) {
            stack.extend(node.children.iter().rev().copied());
        }
    }
    texts
}

fn overlay_input_event(input: &RuntimeSceneInput, last_pointer: LogicalPoint) -> InputEvent {
    let mut event = runtime_input_event(input, last_pointer);
    if let RuntimeSceneInput::Key {
        pressed: true,
        key,
        modifiers,
        ..
    } = input
        && !modifiers.alt
        && !modifiers.control
        && !modifiers.meta
        && key.chars().count() == 1
        && let InputEvent::Keyboard { text, .. } = &mut event
    {
        *text = Some(key.clone());
    }
    event
}

fn context_menu_anchor(state: &GalleryState) -> (f32, f32) {
    let (width, _) = state.gallery_viewport_size();
    let _ = state.context_anchor;
    (width - 24.0, 112.0)
}

fn runtime_context_items(items: &[nana_ui::ContextMenuItem]) -> Vec<RuntimeContextMenuItem> {
    items.to_vec()
}

fn context_action_key(action: ContextAction) -> &'static str {
    match action {
        ContextAction::Duplicate => "duplicate",
        ContextAction::Rename => "rename",
        ContextAction::Remove => "remove",
    }
}

fn context_action_from_value(value: &str) -> Option<ContextAction> {
    let leaf = value.rsplit('/').next().filter(|part| !part.is_empty())?;
    match leaf {
        "rename" => Some(ContextAction::Rename),
        "remove" => Some(ContextAction::Remove),
        "duplicate" => Some(ContextAction::Duplicate),
        _ => None,
    }
}

fn runtime_menu_path(path: &[usize]) -> Vec<Arc<str>> {
    if path.first() == Some(&0) {
        vec![Arc::from(CONTEXT_GROUP)]
    } else {
        Vec::new()
    }
}

fn gallery_menu_path(path: &[Arc<str>]) -> Vec<usize> {
    if path
        .first()
        .is_some_and(|segment| segment.as_ref() == CONTEXT_GROUP)
    {
        vec![0]
    } else {
        Vec::new()
    }
}

fn gallery_command_palette(state: &GalleryState) -> CommandPalette {
    CommandPalette::new(PALETTE_TITLE, state.palette_items())
        .placeholder(PALETTE_PLACEHOLDER)
        .query(state.action_picker.query())
}

fn dialog_body_text() -> Text {
    styled_text(DIALOG_MESSAGE, SemanticColorRole::Text, 13.0, 400)
}

fn mount_image_preview(
    context: &mut AppContext,
    document_id: DocumentId,
) -> Result<Entity<HostStack>, FrameworkError> {
    let stage = context.create_detached_component(
        document_id,
        HostStack::fill_column(0.0).padding(IMAGE_PREVIEW_INSET),
    )?;
    let preview = context.create_detached_component(
        document_id,
        HostStack::fill_column(6.0)
            .align(AlignSpec::Center)
            .background(SemanticColorRole::AccentStrong),
    )?;
    let top = context.create_detached_component(document_id, HostStack::spacer())?;
    let title = context.create_detached_component(
        document_id,
        hugging_text(
            IMAGE_PREVIEW_TITLE,
            SemanticColorRole::AccentText,
            48.0,
            600,
        ),
    )?;
    let caption = context.create_detached_component(
        document_id,
        hugging_text(
            IMAGE_PREVIEW_CAPTION,
            SemanticColorRole::AccentText,
            14.0,
            400,
        ),
    )?;
    let bottom = context.create_detached_component(document_id, HostStack::spacer())?;
    context.append_child(preview, top)?;
    context.append_child(preview, title)?;
    context.append_child(preview, caption)?;
    context.append_child(preview, bottom)?;
    context.append_child(stage, preview)?;
    Ok(stage)
}
