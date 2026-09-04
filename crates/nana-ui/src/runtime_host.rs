//! Backend-neutral application contract for the Nana Scene host.
//!
//! Applications own [`RuntimeDocument`] values and drive [`run_runtime`].
//! [`run_runtime`] is the product host entry and delegates to
//! [`crate::run_runtime_scene`].

use std::fmt;
use std::sync::Arc;
use std::sync::mpsc::{SyncSender, TrySendError};
use std::time::Instant;

use nana_ui_platform::{
    InputEvent, WindowCommand, WindowEvent, WindowGeometry, WindowId, WindowSettings,
};
use nana_ui_runtime::{
    AccessibilityActionRequest, AccessibilityUpdate, AnimationFrame, FrameworkError, StableNodeId,
    Task,
};
use nana_ui_scene::{DocumentAccessError, RuntimeDocument};

use crate::{
    HostTextureRegistry, HostedGpuResources, MaterialOutcome, SceneGpuRendererRegistry, ThemeMode,
};

pub use nana_ui_platform::WindowSettings as RuntimeWindowSettings;

/// Skip the raw program input hook when Runtime already consumed the event.
/// The Scene host does not use this gate; it always delivers `input_event`.
#[cfg(test)]
pub(crate) fn gated_runtime_input_update(
    disposition: nana_ui_platform::InputDisposition,
    id: WindowId,
    raw_input: impl FnOnce() -> Result<RuntimeProgramUpdate, FrameworkError>,
) -> RuntimeProgramUpdate {
    if disposition.prevent_default {
        RuntimeProgramUpdate::redraw(id)
    } else {
        raw_input().unwrap_or_else(|error| panic!("RuntimeProgram input handler failed: {error}"))
    }
}

pub(crate) fn gated_runtime_window_update(
    prevent_raw: bool,
    raw_event: impl FnOnce() -> RuntimeProgramUpdate,
) -> RuntimeProgramUpdate {
    if prevent_raw {
        RuntimeProgramUpdate::default()
    } else {
        raw_event()
    }
}

/// Host services that are safe to retain or invoke from application code.
/// Native window identities intentionally do not cross this boundary.
#[derive(Clone)]
pub struct RuntimeProgramContext<Message: Send + 'static> {
    window_id: WindowId,
    geometry: WindowGeometry,
    gpu: HostedGpuResources,
    material: MaterialOutcome,
    surface_alpha_mode: wgpu::CompositeAlphaMode,
    dispatch: Arc<dyn Fn(Message) + Send + Sync>,
    tasks: SyncSender<Task<Message>>,
}

impl<Message: Send + 'static> RuntimeProgramContext<Message> {
    pub(crate) fn new(
        window_id: WindowId,
        geometry: WindowGeometry,
        gpu: HostedGpuResources,
        material: MaterialOutcome,
        surface_alpha_mode: wgpu::CompositeAlphaMode,
        dispatch: Arc<dyn Fn(Message) + Send + Sync>,
        tasks: SyncSender<Task<Message>>,
    ) -> Self {
        Self {
            window_id,
            geometry,
            gpu,
            material,
            surface_alpha_mode,
            dispatch,
            tasks,
        }
    }

    pub const fn window_id(&self) -> WindowId {
        self.window_id
    }

    pub const fn geometry(&self) -> WindowGeometry {
        self.geometry
    }

    pub fn gpu(&self) -> &HostedGpuResources {
        &self.gpu
    }

    pub const fn material(&self) -> MaterialOutcome {
        self.material
    }

    pub const fn surface_alpha_mode(&self) -> wgpu::CompositeAlphaMode {
        self.surface_alpha_mode
    }

    pub fn dispatch(&self, message: Message) {
        (self.dispatch)(message);
    }

    /// Run a Nana Runtime task on host-owned execution infrastructure and
    /// route its completion back through the native event loop.
    pub fn run_task(&self, task: Task<Message>) -> Result<(), RuntimeTaskError> {
        self.tasks.try_send(task).map_err(|error| match error {
            TrySendError::Full(_) => RuntimeTaskError::QueueFull,
            TrySendError::Disconnected(_) => RuntimeTaskError::HostStopped,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeTaskError {
    QueueFull,
    HostStopped,
}

impl fmt::Display for RuntimeTaskError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::QueueFull => "runtime host task queue is full",
            Self::HostStopped => "runtime host task executor has stopped",
        })
    }
}

impl std::error::Error for RuntimeTaskError {}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum RuntimeRedraw {
    #[default]
    None,
    Window(WindowId),
    Windows(Vec<WindowId>),
    All,
}

impl RuntimeRedraw {
    pub fn for_windows(windows: impl IntoIterator<Item = WindowId>) -> Self {
        let mut windows: Vec<_> = windows.into_iter().collect();
        windows.sort_by_key(|id| id.0);
        windows.dedup();
        match windows.as_slice() {
            [] => Self::None,
            [id] => Self::Window(*id),
            _ => Self::Windows(windows),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuntimeProgramUpdate {
    pub redraw: RuntimeRedraw,
    pub window_commands: Vec<WindowCommand>,
    pub exit: bool,
}

impl RuntimeProgramUpdate {
    pub const fn redraw(id: WindowId) -> Self {
        Self {
            redraw: RuntimeRedraw::Window(id),
            window_commands: Vec::new(),
            exit: false,
        }
    }

    pub const fn redraw_all() -> Self {
        Self {
            redraw: RuntimeRedraw::All,
            window_commands: Vec::new(),
            exit: false,
        }
    }

    pub const fn exit() -> Self {
        Self {
            redraw: RuntimeRedraw::None,
            window_commands: Vec::new(),
            exit: true,
        }
    }

    pub(crate) fn merge(mut self, other: Self) -> Self {
        self.exit |= other.exit;
        self.window_commands.extend(other.window_commands);
        self.redraw = match (self.redraw, other.redraw) {
            (RuntimeRedraw::All, _) | (_, RuntimeRedraw::All) => RuntimeRedraw::All,
            (RuntimeRedraw::None, redraw) | (redraw, RuntimeRedraw::None) => redraw,
            (left, right) => {
                let into_windows = |redraw| match redraw {
                    RuntimeRedraw::Window(id) => vec![id],
                    RuntimeRedraw::Windows(ids) => ids,
                    _ => Vec::new(),
                };
                RuntimeRedraw::for_windows(
                    into_windows(left).into_iter().chain(into_windows(right)),
                )
            }
        };
        self
    }
}

/// A degraded outcome the Scene host recovered from by dropping the failed
/// callback's effect or skipping one frame. Reported through
/// [`RuntimeProgram::host_failure`] so programs decide how to surface the
/// failure instead of the host panicking inside the platform event loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostFailure {
    DocumentAccess { window: WindowId, error: String },
    AccessibilityAction { window: WindowId, error: String },
    ImeDispatch { window: WindowId, error: String },
    AnimationFrame { window: WindowId, error: String },
    InputDispatch { window: WindowId, error: String },
    InputHandler { window: WindowId, error: String },
    MissingDocument { window: WindowId },
    FrameDidNotSettle { window: WindowId, error: String },
    ResourceProduction { window: WindowId, error: String },
    UnpaintableScene { window: WindowId, error: String },
    AuxiliarySurfaceLost { window: WindowId },
}

impl HostFailure {
    pub fn window(&self) -> WindowId {
        match self {
            Self::DocumentAccess { window, .. }
            | Self::AccessibilityAction { window, .. }
            | Self::ImeDispatch { window, .. }
            | Self::AnimationFrame { window, .. }
            | Self::InputDispatch { window, .. }
            | Self::InputHandler { window, .. }
            | Self::MissingDocument { window }
            | Self::FrameDidNotSettle { window, .. }
            | Self::ResourceProduction { window, .. }
            | Self::UnpaintableScene { window, .. }
            | Self::AuxiliarySurfaceLost { window } => *window,
        }
    }

    pub fn error(&self) -> Option<&str> {
        match self {
            Self::DocumentAccess { error, .. }
            | Self::AccessibilityAction { error, .. }
            | Self::ImeDispatch { error, .. }
            | Self::AnimationFrame { error, .. }
            | Self::InputDispatch { error, .. }
            | Self::InputHandler { error, .. }
            | Self::FrameDidNotSettle { error, .. }
            | Self::ResourceProduction { error, .. }
            | Self::UnpaintableScene { error, .. } => Some(error),
            Self::MissingDocument { .. } | Self::AuxiliarySurfaceLost { .. } => None,
        }
    }
}

impl fmt::Display for HostFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "host failure on window {}", self.window().0)?;
        match self {
            Self::DocumentAccess { .. } => formatter.write_str(": document access failed"),
            Self::AccessibilityAction { .. } => {
                formatter.write_str(": accessibility action failed")
            }
            Self::ImeDispatch { .. } => formatter.write_str(": IME dispatch failed"),
            Self::AnimationFrame { .. } => formatter.write_str(": animation handler failed"),
            Self::InputDispatch { .. } => formatter.write_str(": input dispatch failed"),
            Self::InputHandler { .. } => formatter.write_str(": input handler failed"),
            Self::MissingDocument { .. } => formatter.write_str(": no document for window"),
            Self::FrameDidNotSettle { .. } => formatter.write_str(": frame did not settle"),
            Self::ResourceProduction { .. } => formatter.write_str(": resource production failed"),
            Self::UnpaintableScene { .. } => formatter.write_str(": unpaintable UiScene"),
            Self::AuxiliarySurfaceLost { .. } => {
                formatter.write_str(": auxiliary surface closed during frame")
            }
        }
    }
}

/// Canonical retained application contract for the Nana Scene host.
///
/// `Message` is for host-level work (windows, GPU, persistence). Control
/// interaction should update Runtime views through `on` / `observe`.
pub trait RuntimeProgram: Sized + 'static {
    type Message: Send + 'static;
    type Error: fmt::Display;

    fn initialize(
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<(Self, Vec<Self::Message>), Self::Error>;

    /// Borrow one document for a synchronous operation. References cannot escape
    /// the callback; release the scope before invoking application or JS code.
    fn with_document<R>(
        &self,
        id: WindowId,
        f: impl FnOnce(&RuntimeDocument) -> R,
    ) -> Result<Option<R>, DocumentAccessError>;
    fn with_document_mut<R>(
        &mut self,
        id: WindowId,
        f: impl FnOnce(&mut RuntimeDocument) -> R,
    ) -> Result<Option<R>, DocumentAccessError>;

    /// Apply a host-level message. `dispatch_program` coalesces to the latest
    /// and runs on the next frame. Keep this cheap; fill content in
    /// [`Self::bind_window`] after present.
    fn update(
        &mut self,
        message: Self::Message,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate;

    fn theme_mode(&self) -> ThemeMode;

    fn window_material_mode(&self) -> crate::MaterialEffect {
        crate::MaterialEffect::Solid
    }

    fn appearance_backdrop_opacity(&self) -> f32 {
        nana_ui_core::AppearanceSettings::DEFAULT_BACKDROP_OPACITY
    }

    /// Product default: attach an existing sampleable texture to the tree.
    ///
    /// Pair with [`crate::GpuTextureView`] on the same slot, then update the
    /// view in [`Self::prepare_window_frame`].
    fn host_textures(&self, _id: WindowId) -> Option<HostTextureRegistry> {
        None
    }

    /// Advanced: encode into the current UI pass. Prefer [`Self::host_textures`].
    ///
    /// Returning `None` lets the hosted runtime attach a demo `"gpu-view"`
    /// painter that uses stored host Device/Queue clones. `Some(registry)` is
    /// used unchanged. [`crate::SceneWgpuPainter`] consumes the resolved
    /// registry; an explicit empty registry leaves `"gpu-view"` unpaintable.
    fn scene_gpu_renderers(&self, _id: WindowId) -> Option<SceneGpuRendererRegistry> {
        None
    }

    /// Advanced: graph-scheduled offscreen on the HostTexture path.
    /// Prefer [`Self::prepare_window_frame`]. Same Device/Queue; submit before
    /// Scene samples.
    fn scene_resource_producers(
        &self,
        _id: WindowId,
    ) -> Option<crate::SceneResourceProducerRegistry> {
        None
    }

    /// Acquire application-owned frame resources immediately before the host
    /// flushes and paints this window. Resources retired here must remain alive
    /// until [`Self::window_frame_presented`] confirms Surface submission.
    fn prepare_window_frame(
        &mut self,
        _id: WindowId,
        _context: &RuntimeProgramContext<Self::Message>,
    ) {
    }

    /// Release resources retired by [`Self::prepare_window_frame`] only after
    /// the host has submitted and presented this window's frame.
    fn window_frame_presented(
        &mut self,
        _id: WindowId,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        RuntimeProgramUpdate::default()
    }

    /// Fill content after the host presented a frame that applied [`Self::update`].
    fn bind_window(
        &mut self,
        _id: WindowId,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        RuntimeProgramUpdate::default()
    }

    fn rebuild_gpu(&mut self, _context: &RuntimeProgramContext<Self::Message>) {}

    /// Report a [`HostFailure`] the host has already recovered from: the
    /// failed callback's effect was dropped or the frame was skipped, and the
    /// event loop keeps running. Override to log, surface UI feedback, or
    /// exit; the default ignores the report.
    fn host_failure(&mut self, _failure: HostFailure) {}

    fn input_event(
        &mut self,
        _id: WindowId,
        _event: &InputEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        Ok(RuntimeProgramUpdate::default())
    }

    /// Receive an input event together with the topmost interactive node the
    /// host hit-tested under the pointer. `pointer_hit` is `Some` only for
    /// [`InputEvent::Pointer`] and [`InputEvent::Wheel`]. Override this
    /// instead of [`Self::input_event`] when the program routes raw input to
    /// hosted surfaces; the default forwards unchanged so existing programs
    /// keep working.
    fn input_event_routed(
        &mut self,
        id: WindowId,
        event: &InputEvent,
        _pointer_hit: Option<StableNodeId>,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        self.input_event(id, event, context)
    }

    fn window_event(
        &mut self,
        _event: WindowEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        RuntimeProgramUpdate::default()
    }

    /// Application-owned wake deadline for sampled state, external runtimes,
    /// retry backoff or other work that must not depend on UI redraw cadence.
    fn next_wakeup(&self) -> Option<Instant> {
        None
    }

    fn wake(
        &mut self,
        _now: Instant,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate {
        RuntimeProgramUpdate::default()
    }

    /// Align hosted program CSS animation sampling with the Scene host epoch.
    fn sync_animation_clock(&mut self, _epoch: Instant) {}

    fn animation_frame(
        &mut self,
        _id: WindowId,
        _frame: AnimationFrame,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        Ok(RuntimeProgramUpdate::default())
    }

    /// Drain accessibility work already applied to the retained world.
    /// Used when `RuntimeDocument::flush` is empty because a consumer flushed
    /// systems earlier in the same frame.
    fn take_accessibility_update(&mut self, _id: WindowId) -> Option<AccessibilityUpdate> {
        None
    }

    fn accessibility_action(
        &mut self,
        id: WindowId,
        request: AccessibilityActionRequest,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        let changed = self
            .with_document_mut(id, |document| {
                let document_id = document.document();
                document
                    .context_mut()
                    .apply_accessibility_action(document_id, request)
            })
            .map_err(|error| {
                self.host_failure(HostFailure::DocumentAccess {
                    window: id,
                    error: error.to_string(),
                });
                FrameworkError::InvalidInput
            })?
            .transpose()?
            .unwrap_or(false);
        Ok(if changed {
            RuntimeProgramUpdate::redraw(id)
        } else {
            RuntimeProgramUpdate::default()
        })
    }
}

/// Host boundary: report failed access after the document scope has ended.
pub(crate) trait HostDocumentAccess: RuntimeProgram {
    fn read_document<R>(
        &mut self,
        id: WindowId,
        f: impl FnOnce(&RuntimeDocument) -> R,
    ) -> Option<R> {
        match self.with_document(id, f) {
            Ok(value) => value,
            Err(error) => {
                self.host_failure(HostFailure::DocumentAccess {
                    window: id,
                    error: error.to_string(),
                });
                None
            }
        }
    }
    fn write_document<R>(
        &mut self,
        id: WindowId,
        f: impl FnOnce(&mut RuntimeDocument) -> R,
    ) -> Option<R> {
        match self.with_document_mut(id, f) {
            Ok(value) => value,
            Err(error) => {
                self.host_failure(HostFailure::DocumentAccess {
                    window: id,
                    error: error.to_string(),
                });
                None
            }
        }
    }
}
impl<T: RuntimeProgram> HostDocumentAccess for T {}

pub(crate) fn runtime_text_input_request(
    document: &RuntimeDocument,
) -> nana_ui_platform::TextInputRequest {
    let focused = document
        .context()
        .focused_text_input(document.document())
        .map(|(target, _)| target)
        .filter(|target| {
            document
                .context()
                .world()
                .accessibility(*target)
                .is_some_and(|state| state.editable)
        });
    let cursor_area = focused
        .and_then(
            |target| match document.context().world().component_geometry(target) {
                Some(nana_ui_runtime::ComponentGeometry::TextInput {
                    caret: Some(caret), ..
                }) => Some(caret),
                _ => document.context().world().layout_box(target),
            },
        )
        .map(|layout| {
            nana_ui_core::LogicalRect::new(layout.x, layout.y, layout.width, layout.height)
        });
    let secure = focused
        .and_then(|target| document.context().world().standard_visual(target))
        .is_some_and(|visual| {
            matches!(
                visual,
                nana_ui_runtime::StandardVisual::TextInput { secure: true, .. }
            )
        });
    let purpose = if secure {
        nana_ui_platform::TextInputPurpose::Password
    } else {
        nana_ui_platform::TextInputPurpose::Normal
    };
    nana_ui_platform::TextInputRequest {
        enabled: focused.is_some(),
        cursor_area,
        purpose,
    }
}

/// Focused editor excerpt for winit surrounding text. Password / unfocused: `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImeSurroundingSnapshot {
    pub text: String,
    pub cursor: usize,
    pub anchor: usize,
}

const IME_SURROUNDING_MAX_BYTES: usize = 4000;

pub(crate) fn runtime_ime_surrounding(
    document: &RuntimeDocument,
) -> Option<ImeSurroundingSnapshot> {
    let request = runtime_text_input_request(document);
    if !request.enabled || request.purpose == nana_ui_platform::TextInputPurpose::Password {
        return None;
    }
    let (_, state) = document.context().focused_text_input(document.document())?;
    clip_ime_surrounding(&state.value, state.selection.focus, state.selection.anchor)
}

fn clip_ime_surrounding(
    text: &str,
    cursor: usize,
    anchor: usize,
) -> Option<ImeSurroundingSnapshot> {
    if !text.is_char_boundary(cursor) || !text.is_char_boundary(anchor) {
        return None;
    }
    if text.len() <= IME_SURROUNDING_MAX_BYTES {
        return Some(ImeSurroundingSnapshot {
            text: text.to_string(),
            cursor,
            anchor,
        });
    }
    let start = text.floor_char_boundary(cursor.saturating_sub(IME_SURROUNDING_MAX_BYTES / 2));
    let end = text.floor_char_boundary((start + IME_SURROUNDING_MAX_BYTES).min(text.len()));
    if end <= start {
        return None;
    }
    Some(ImeSurroundingSnapshot {
        text: text[start..end].to_string(),
        cursor: cursor.saturating_sub(start).min(end - start),
        anchor: anchor.saturating_sub(start).min(end - start),
    })
}

pub fn run_runtime<Program: RuntimeProgram>(
    settings: WindowSettings,
) -> Result<(), crate::HostedRunError> {
    crate::run_runtime_scene::<Program>(settings)
}

#[cfg(test)]
mod tests {
    use super::{
        IME_SURROUNDING_MAX_BYTES, clip_ime_surrounding, gated_runtime_input_update,
        gated_runtime_window_update, runtime_ime_surrounding, runtime_text_input_request,
    };
    use nana_ui_platform::{InputDisposition, InputEvent, InputModifiers, WindowId};
    use nana_ui_runtime::{AppContext, Dialog, DocumentId, OverlayHost, TextArea};

    #[test]
    fn runtime_ime_request_uses_editability_and_secure_purpose() {
        let document_id = nana_ui_runtime::DocumentId::new(1).unwrap();
        let mut document = nana_ui_scene::RuntimeDocument::new(document_id);
        let input = document
            .context_mut()
            .create_component(
                document_id,
                nana_ui_runtime::TextInput::new("secret").secure(true),
            )
            .unwrap();
        assert!(
            document
                .context_mut()
                .focus_node(document_id, input.stable_id())
                .unwrap()
        );

        let request = runtime_text_input_request(&document);
        assert!(request.enabled);
        assert_eq!(
            request.purpose,
            nana_ui_platform::TextInputPurpose::Password
        );

        document
            .context_mut()
            .update_component(input, |input, _cx| input.read_only = true)
            .unwrap();
        let request = runtime_text_input_request(&document);
        assert!(!request.enabled);
        assert_eq!(request.purpose, nana_ui_platform::TextInputPurpose::Normal);
    }

    #[test]
    fn runtime_textarea_ime_request_follows_focus_and_normal_purpose() {
        let document_id = nana_ui_runtime::DocumentId::new(1).unwrap();
        let mut document = nana_ui_scene::RuntimeDocument::new(document_id);
        let area = document
            .context_mut()
            .create_component(document_id, TextArea::new("第一行\n第二行"))
            .unwrap();

        let request = runtime_text_input_request(&document);
        assert!(!request.enabled);
        assert_eq!(request.purpose, nana_ui_platform::TextInputPurpose::Normal);
        assert!(request.cursor_area.is_none());

        assert!(
            document
                .context_mut()
                .focus_node(document_id, area.stable_id())
                .unwrap()
        );
        let request = runtime_text_input_request(&document);
        assert!(request.enabled);
        assert_eq!(request.purpose, nana_ui_platform::TextInputPurpose::Normal);

        document
            .context_mut()
            .update_component(area, |area, _cx| area.disabled = true)
            .unwrap();
        let request = runtime_text_input_request(&document);
        assert!(!request.enabled);
        assert_eq!(request.purpose, nana_ui_platform::TextInputPurpose::Normal);
    }

    #[test]
    fn runtime_ime_surrounding_follows_focus_and_skips_password() {
        let document_id = nana_ui_runtime::DocumentId::new(1).unwrap();
        let mut document = nana_ui_scene::RuntimeDocument::new(document_id);
        let input = document
            .context_mut()
            .create_component(document_id, nana_ui_runtime::TextInput::new("NanaUI"))
            .unwrap();
        assert!(runtime_ime_surrounding(&document).is_none());

        assert!(
            document
                .context_mut()
                .focus_node(document_id, input.stable_id())
                .unwrap()
        );
        let surrounding = runtime_ime_surrounding(&document).expect("focused editor");
        assert_eq!(surrounding.text, "NanaUI");
        assert_eq!(surrounding.cursor, "NanaUI".len());
        assert_eq!(surrounding.anchor, "NanaUI".len());

        let password = document
            .context_mut()
            .create_component(
                document_id,
                nana_ui_runtime::TextInput::new("secret").secure(true),
            )
            .unwrap();
        assert!(
            document
                .context_mut()
                .focus_node(document_id, password.stable_id())
                .unwrap()
        );
        assert!(runtime_ime_surrounding(&document).is_none());
    }

    #[test]
    fn clip_ime_surrounding_stays_within_winit_limit() {
        let text = "字".repeat(IME_SURROUNDING_MAX_BYTES);
        let caret = text.len();
        let clip = clip_ime_surrounding(&text, caret, caret).expect("excerpt");
        assert!(clip.text.len() <= IME_SURROUNDING_MAX_BYTES);
        assert!(clip.text.is_char_boundary(clip.cursor));
        assert!(clip.text.is_char_boundary(clip.anchor));
    }

    #[test]
    fn consumed_runtime_input_never_reaches_the_raw_program_handler() {
        let mut called = false;
        let _ = gated_runtime_input_update(
            InputDisposition {
                prevent_default: true,
            },
            WindowId::PRIMARY,
            || {
                called = true;
                Ok(super::RuntimeProgramUpdate::default())
            },
        );
        assert!(!called);

        let _ = gated_runtime_input_update(
            InputDisposition {
                prevent_default: false,
            },
            WindowId::PRIMARY,
            || {
                called = true;
                Ok(super::RuntimeProgramUpdate::default())
            },
        );
        assert!(called);
    }

    #[test]
    fn blocking_overlay_primary_tab_never_reaches_the_raw_program_handler() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let host = context
            .create_component(document, OverlayHost::new())
            .unwrap();
        let dialog = context
            .create_component(document, Dialog::new("Settings"))
            .unwrap();
        context.append_child(host, dialog).unwrap();
        context.activate_overlay(host, dialog).unwrap();
        let disposition = crate::RuntimeInputAdapter::default()
            .dispatch(
                &mut context,
                document,
                &InputEvent::Keyboard {
                    pressed: true,
                    key: "Tab".into(),
                    text: None,
                    code: "Tab".into(),
                    repeat: false,
                    modifiers: InputModifiers {
                        control: true,
                        ..InputModifiers::default()
                    },
                },
            )
            .unwrap();

        let mut calls = 0;
        let _ = gated_runtime_input_update(disposition, WindowId::PRIMARY, || {
            calls += 1;
            Ok(super::RuntimeProgramUpdate::default())
        });
        assert_eq!(calls, 0);
    }

    #[test]
    fn gated_window_update_skips_the_raw_handler_only_when_asked() {
        let mut calls = 0;
        let _ = gated_runtime_window_update(true, || {
            calls += 1;
            super::RuntimeProgramUpdate::default()
        });
        assert_eq!(calls, 0);

        let _ = gated_runtime_window_update(false, || {
            calls += 1;
            super::RuntimeProgramUpdate::default()
        });
        assert_eq!(calls, 1);
    }
}
