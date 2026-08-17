use std::fmt;
use std::sync::{Arc, Mutex};

use iced::widget::space;
use iced::{Element, Event, Length, Point, Size};
#[cfg(test)]
use nana_ui::runtime::StandardVisual;
use nana_ui::runtime::{
    AlignSpec, AppContext, Button, CommandPalette, ConfirmDialog, ConfirmIntent, ConfirmSlots,
    ContextMenu, ContextMenuEvent as RuntimeContextMenuEvent,
    ContextMenuItem as RuntimeContextMenuItem, DocumentId, Entity, FrameworkError, IconButton,
    ImageViewer, ImageViewerContent, ImageViewerEvent, LayoutBox, LayoutViewport, OverlayChanged,
    OverlayHost, RuntimeDocument, SemanticColorRole, Text,
};
use nana_ui::{
    ButtonKind, CommandPaletteEvent, ControlSize, IcedSceneView, IcedTextShaper, Icon,
    RuntimeInputAdapter,
};
use nana_ui_platform::{InputEvent, PointerPhase};

use super::runtime_host::{
    HostStack, RuntimeSceneInput, bind_event, hugging_text, iced_key_name, iced_modifiers,
    runtime_input_event, scene_pointer, styled_text, take_pending,
};
use super::{
    ContextAction, ContextMenuEvent, DialogCloseTrigger, GalleryMessage, GalleryOverlay,
    GalleryState,
};

const OVERLAY_DOCUMENT: u64 = 3;
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
    document: RuntimeDocument,
    kind: GalleryOverlay,
    #[allow(dead_code)]
    host: Entity<OverlayHost>,
    palette: Option<Entity<CommandPalette>>,
    #[allow(dead_code)]
    dialog: Option<Entity<ConfirmDialog>>,
    image: Option<Entity<ImageViewer>>,
    menu: Option<Entity<ContextMenu>>,
    last_menu_path: Vec<usize>,
    last_pointer: Point,
    last_viewport: LayoutViewport,
    pending: Arc<Mutex<Vec<GalleryMessage>>>,
}

impl fmt::Debug for GalleryOverlaysRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GalleryOverlaysRuntime")
            .field("kind", &self.kind)
            .field("last_viewport", &self.last_viewport)
            .finish_non_exhaustive()
    }
}

impl GalleryOverlaysRuntime {
    fn mount(state: &GalleryState, kind: GalleryOverlay) -> Result<Self, FrameworkError> {
        let pending = Arc::new(Mutex::new(Vec::new()));
        let mut document =
            RuntimeDocument::new(DocumentId::new(OVERLAY_DOCUMENT).expect("overlay document id"));
        let document_id = document.document();
        let context = document.context_mut();
        let _ = context.set_theme(state.theme);

        let host = context.create_component(document_id, OverlayHost::new())?;
        let mut palette = None;
        let mut dialog = None;
        let mut image = None;
        let mut menu = None;

        match kind {
            GalleryOverlay::CommandPalette => {
                let overlay = context
                    .create_detached_component(document_id, gallery_command_palette(state))?;
                context.append_child(host, overlay)?;
                context.activate_overlay(host, overlay)?;
                bind_event(
                    context,
                    overlay,
                    Arc::clone(&pending),
                    |event: &CommandPaletteEvent| match event {
                        CommandPaletteEvent::Navigate(_) => GalleryMessage::OverlayInteraction,
                        event => GalleryMessage::CommandPalette(event.clone()),
                    },
                )?;
                palette = Some(overlay);
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
                context.append_child(host, overlay)?;
                context.activate_overlay(host, overlay)?;
                bind_event(
                    context,
                    overlay,
                    Arc::clone(&pending),
                    |intent: &ConfirmIntent| match intent {
                        ConfirmIntent::Confirm { .. } => GalleryMessage::ConfirmDialog,
                        ConfirmIntent::Cancel | ConfirmIntent::Secondary => {
                            GalleryMessage::RequestDialogClose(DialogCloseTrigger::CloseButton)
                        }
                    },
                )?;
                dialog = Some(overlay);
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
                context.append_child(host, overlay)?;
                context.activate_overlay(host, overlay)?;
                bind_event(
                    context,
                    overlay,
                    Arc::clone(&pending),
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
                image = Some(overlay);
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
                context.append_child(host, overlay)?;
                context.activate_overlay(host, overlay)?;
                bind_event(
                    context,
                    overlay,
                    Arc::clone(&pending),
                    |event: &RuntimeContextMenuEvent| match event {
                        RuntimeContextMenuEvent::Search(query) => {
                            GalleryMessage::ContextMenu(ContextMenuEvent::Search(query.to_string()))
                        }
                        RuntimeContextMenuEvent::Select(value) => {
                            match context_action_from_value(value) {
                                Some(action) => {
                                    GalleryMessage::ContextMenu(ContextMenuEvent::Select(action))
                                }
                                None => GalleryMessage::OverlayInteraction,
                            }
                        }
                        RuntimeContextMenuEvent::Dismiss => {
                            GalleryMessage::ContextMenu(ContextMenuEvent::Dismiss)
                        }
                    },
                )?;
                menu = Some(overlay);
            }
        }

        bind_event(
            context,
            host,
            Arc::clone(&pending),
            move |event: &OverlayChanged| {
                if event.active.is_none() {
                    overlay_dismissed_message(kind)
                } else {
                    GalleryMessage::OverlayInteraction
                }
            },
        )?;

        let (width, height) = state.gallery_viewport_size();
        let last_viewport = LayoutViewport::new(width, height);
        let _ = document.flush(last_viewport, &mut IcedTextShaper);

        Ok(Self {
            document,
            kind,
            host,
            palette,
            dialog,
            image,
            menu,
            last_menu_path: state.context_path.clone(),
            last_pointer: Point::ORIGIN,
            last_viewport,
            pending,
        })
    }

    fn sync(&mut self, state: &GalleryState) {
        let context = self.document.context_mut();
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
        self.flush(state.gallery_viewport_size());
    }

    fn flush(&mut self, (width, height): (f32, f32)) {
        self.last_viewport = LayoutViewport::new(width, height);
        let _ = self.document.flush(self.last_viewport, &mut IcedTextShaper);
    }

    fn dispatch(&mut self, event: InputEvent) -> Vec<GalleryMessage> {
        if let InputEvent::Pointer { x, y, .. } | InputEvent::Wheel { x, y, .. } = event {
            self.last_pointer = Point::new(x, y);
        }
        if let Some(viewer) = self.image {
            let context = self.document.context_mut();
            match event {
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
        let document = self.document.document();
        let _ =
            RuntimeInputAdapter::default().dispatch(self.document.context_mut(), document, &event);
        let mut messages = take_pending(&self.pending);
        if let Some(menu) = self.menu
            && let Ok(path) = self
                .document
                .context()
                .read(menu, |menu| menu.active_path.clone())
        {
            let gallery_path = gallery_menu_path(&path);
            if gallery_path != self.last_menu_path {
                self.last_menu_path = gallery_path.clone();
                messages.push(GalleryMessage::ContextMenu(ContextMenuEvent::OpenSubmenu(
                    gallery_path,
                )));
            }
        }
        messages
    }

    fn viewport_size(&self) -> Size {
        Size::new(self.last_viewport.width, self.last_viewport.height)
    }

    #[cfg(test)]
    fn scene_populated(&self) -> bool {
        !self.document.scene().is_empty()
    }
}

impl GalleryState {
    pub(super) fn refresh_overlay_runtime(&mut self) {
        let Some(kind) = self.overlay.active().copied() else {
            self.overlay_runtime = None;
            return;
        };
        let remount = self
            .overlay_runtime
            .as_ref()
            .is_none_or(|runtime| runtime.kind != kind);
        if remount {
            match GalleryOverlaysRuntime::mount(self, kind) {
                Ok(runtime) => self.overlay_runtime = Some(runtime),
                Err(_) => {
                    self.overlay_runtime = None;
                    return;
                }
            }
        }
        if let Some(mut runtime) = self.overlay_runtime.take() {
            runtime.sync(self);
            self.overlay_runtime = Some(runtime);
        }
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
        let messages = runtime.dispatch(event);
        self.overlay_runtime = Some(runtime);
        for message in messages {
            self.update(message);
        }
        if self.overlay.is_open() {
            self.refresh_overlay_runtime();
        } else {
            self.overlay_runtime = None;
        }
    }

    pub(super) fn overlay_runtime_view(&self) -> Element<'_, GalleryMessage> {
        let Some(runtime) = self.overlay_runtime.as_ref() else {
            return space().width(Length::Fill).height(Length::Fill).into();
        };
        let view = match IcedSceneView::new(runtime.document.scene(), runtime.viewport_size()) {
            Ok(view) => Element::from(view),
            Err(_) => space().width(Length::Fill).height(Length::Fill).into(),
        };
        scene_pointer(
            view,
            iced::mouse::Interaction::None,
            GalleryMessage::OverlayRuntime,
        )
    }

    #[cfg(test)]
    pub(crate) fn gallery_overlay_runtime_scene_populated(&self) -> bool {
        self.overlay_runtime
            .as_ref()
            .is_some_and(GalleryOverlaysRuntime::scene_populated)
    }

    #[cfg(test)]
    pub(crate) fn gallery_overlay_command_palette_state(
        &self,
    ) -> Option<(String, String, String, usize)> {
        let runtime = self.overlay_runtime.as_ref()?;
        let palette = runtime.palette?;
        runtime
            .document
            .context()
            .read(palette, |palette| {
                (
                    palette.title.to_string(),
                    palette.query.clone(),
                    palette.state.value.clone(),
                    palette.selected,
                )
            })
            .ok()
    }

    #[cfg(test)]
    pub(crate) fn gallery_overlay_command_palette_visual(&self) -> Option<(String, String)> {
        let runtime = self.overlay_runtime.as_ref()?;
        let palette = runtime.palette?;
        match runtime
            .document
            .context()
            .world()
            .standard_visual(palette.stable_id())?
        {
            StandardVisual::CommandPalette { title, query, .. } => {
                Some((title.to_string(), query.to_string()))
            }
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn gallery_overlay_dialog_copy(&self) -> Option<(String, String)> {
        let runtime = self.overlay_runtime.as_ref()?;
        let dialog = runtime.dialog?;
        runtime
            .document
            .context()
            .read(dialog, |dialog| {
                (dialog.title.to_string(), dialog.message.to_string())
            })
            .ok()
    }

    #[cfg(test)]
    pub(crate) fn gallery_overlay_image_preview(&self) -> Option<(bool, Vec<String>)> {
        let runtime = self.overlay_runtime.as_ref()?;
        let viewer = runtime.image?;
        let content = runtime
            .document
            .context()
            .read(viewer, |viewer| viewer.content.clone())
            .ok()?;
        let ImageViewerContent::Child(child) = content else {
            return Some((false, Vec::new()));
        };
        Some((true, collect_node_text(runtime.document.context(), child)))
    }
}

#[cfg(test)]
pub(crate) fn gallery_context_action_from_value(value: &str) -> Option<ContextAction> {
    context_action_from_value(value)
}

#[cfg(test)]
pub(crate) fn gallery_runtime_context_item_icons(
    items: &[nana_ui::components::ContextMenuItem<'static, ContextAction>],
) -> Vec<(String, String, Option<Icon>)> {
    runtime_context_items(items)
        .into_iter()
        .map(|item| (item.value.to_string(), item.label.to_string(), item.icon))
        .collect()
}

#[cfg(test)]
fn collect_node_text(context: &AppContext, root: nana_ui::runtime::StableNodeId) -> Vec<String> {
    let mut texts = Vec::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if let Some(text) = context.world().text(id) {
            if !text.is_empty() {
                texts.push(text.to_owned());
            }
        }
        if let Some(node) = context.world().node(id) {
            stack.extend(node.children.iter().rev().copied());
        }
    }
    texts
}

pub(super) fn overlay_runtime_key_event(
    event: Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<GalleryMessage> {
    match event {
        Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key,
            modifiers,
            repeat,
            ..
        }) => Some(GalleryMessage::OverlayRuntime(RuntimeSceneInput::Key {
            pressed: true,
            key: iced_key_name(&key)?,
            repeat,
            modifiers: iced_modifiers(modifiers),
        })),
        Event::Keyboard(iced::keyboard::Event::KeyReleased { key, modifiers, .. }) => {
            Some(GalleryMessage::OverlayRuntime(RuntimeSceneInput::Key {
                pressed: false,
                key: iced_key_name(&key)?,
                repeat: false,
                modifiers: iced_modifiers(modifiers),
            }))
        }
        _ => None,
    }
}

fn overlay_input_event(input: &RuntimeSceneInput, last_pointer: Point) -> InputEvent {
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

fn overlay_dismissed_message(kind: GalleryOverlay) -> GalleryMessage {
    match kind {
        GalleryOverlay::CommandPalette => {
            GalleryMessage::CommandPalette(CommandPaletteEvent::Dismiss)
        }
        GalleryOverlay::Dialog => GalleryMessage::RequestDialogClose(DialogCloseTrigger::Outside),
        GalleryOverlay::ImageViewer => {
            GalleryMessage::RequestImageViewerClose(DialogCloseTrigger::Outside)
        }
        GalleryOverlay::ContextMenu => GalleryMessage::ContextMenu(ContextMenuEvent::Dismiss),
    }
}

fn context_menu_anchor(state: &GalleryState) -> (f32, f32) {
    let (width, _) = state.gallery_viewport_size();
    let _ = state.context_anchor;
    (width - 24.0, 112.0)
}

fn runtime_context_items(
    items: &[nana_ui::components::ContextMenuItem<'static, ContextAction>],
) -> Vec<RuntimeContextMenuItem> {
    fn walk(
        items: &[nana_ui::components::ContextMenuItem<'static, ContextAction>],
        prefix: Option<&str>,
        out: &mut Vec<RuntimeContextMenuItem>,
    ) {
        for item in items {
            let segment = context_item_segment(item);
            let value = match prefix {
                Some(prefix) => format!("{prefix}/{segment}"),
                None => segment.to_string(),
            };
            let mut runtime = RuntimeContextMenuItem::new(value.clone(), item.label.to_string())
                .disabled(item.disabled)
                .danger(item.danger);
            if let Some(icon) = item.icon {
                runtime = runtime.icon(icon);
            }
            out.push(runtime);
            if !item.children.is_empty() {
                walk(&item.children, Some(&value), out);
            }
        }
    }
    let mut out = Vec::new();
    walk(items, None, &mut out);
    out
}

fn context_item_segment(
    item: &nana_ui::components::ContextMenuItem<'static, ContextAction>,
) -> &'static str {
    if item.children.is_empty() {
        context_action_key(item.value)
    } else {
        CONTEXT_GROUP
    }
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
