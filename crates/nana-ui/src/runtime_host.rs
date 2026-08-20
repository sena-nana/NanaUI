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
    AccessibilityActionRequest, AccessibilityUpdate, AnimationFrame, FrameworkError, Task,
};
use nana_ui_scene::RuntimeDocument;

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
    dispatch: Arc<dyn Fn(Message) + Send + Sync>,
    tasks: SyncSender<Task<Message>>,
}

impl<Message: Send + 'static> RuntimeProgramContext<Message> {
    pub(crate) fn new(
        window_id: WindowId,
        geometry: WindowGeometry,
        gpu: HostedGpuResources,
        material: MaterialOutcome,
        dispatch: Arc<dyn Fn(Message) + Send + Sync>,
        tasks: SyncSender<Task<Message>>,
    ) -> Self {
        Self {
            window_id,
            geometry,
            gpu,
            material,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RuntimeRedraw {
    #[default]
    None,
    Window(WindowId),
    All,
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
            (RuntimeRedraw::Window(left), RuntimeRedraw::Window(right)) if left == right => {
                RuntimeRedraw::Window(left)
            }
            _ => RuntimeRedraw::All,
        };
        self
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

    fn document(&self, id: WindowId) -> Option<&RuntimeDocument>;
    fn document_mut(&mut self, id: WindowId) -> Option<&mut RuntimeDocument>;

    fn update(
        &mut self,
        message: Self::Message,
        context: &RuntimeProgramContext<Self::Message>,
    ) -> RuntimeProgramUpdate;

    fn theme_mode(&self) -> ThemeMode;

    fn host_textures(&self, _id: WindowId) -> Option<HostTextureRegistry> {
        None
    }

    /// Advanced direct Scene renderers executed with NanaUI's current frame
    /// encoder and render target. HostTexture remains available as a simpler
    /// compatibility resource path.
    ///
    /// Returning `None` lets the hosted runtime attach a default `"gpu-view"`
    /// painter that uses stored host Device/Queue clones. `Some(registry)` is
    /// used unchanged. [`crate::SceneWgpuPainter`] consumes the resolved
    /// registry; an explicit empty registry leaves `"gpu-view"` unpaintable.
    fn scene_gpu_renderers(&self, _id: WindowId) -> Option<SceneGpuRendererRegistry> {
        None
    }

    /// External texture producers encoded by the host before UiScene samples
    /// their resources. The queue submission remains ordered ahead of Scene
    /// presentation. The host submits the same Device/Queue pair.
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

    fn rebuild_gpu(&mut self, _context: &RuntimeProgramContext<Self::Message>) {}

    fn input_event(
        &mut self,
        _id: WindowId,
        _event: &InputEvent,
        _context: &RuntimeProgramContext<Self::Message>,
    ) -> Result<RuntimeProgramUpdate, FrameworkError> {
        Ok(RuntimeProgramUpdate::default())
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
            .document_mut(id)
            .map(|document| {
                let document_id = document.document();
                document
                    .context_mut()
                    .apply_accessibility_action(document_id, request)
            })
            .transpose()?
            .unwrap_or(false);
        Ok(if changed {
            RuntimeProgramUpdate::redraw(id)
        } else {
            RuntimeProgramUpdate::default()
        })
    }
}

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

pub fn run_runtime<Program: RuntimeProgram>(
    settings: WindowSettings,
) -> Result<(), crate::HostedRunError> {
    crate::run_runtime_scene::<Program>(settings)
}

#[cfg(test)]
mod tests {
    use super::{
        gated_runtime_input_update, gated_runtime_window_update, runtime_text_input_request,
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
