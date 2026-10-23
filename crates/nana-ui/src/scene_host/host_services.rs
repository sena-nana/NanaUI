//! A native window as an input source: its endpoint and stamps, and the
//! host services the Runtime drives while routing its input. The IME state
//! is recorded while routing and applied to the window at the end of the
//! turn, the clipboard is the process's one backend, and the cursor the Runtime asks for is composed here with the
//! window's own frame edges and the program's override.

use std::sync::OnceLock;

use nana_ui_platform::{
    EndpointGeneration, HostServiceError, HostServices, InputEndpoint, InputSequencer,
    InputSourceId, SharedClipboardHost, TextInputContext, TextInputPurpose,
    default_shared_clipboard, read_shared_clipboard, read_shared_clipboard_token,
    write_shared_clipboard, write_shared_clipboard_with_token,
};
use winit::cursor::CursorIcon;
use winit::window::{
    ImeCapabilities, ImeEnableRequest, ImeHint, ImePurpose, ImeRequest, ImeRequestData,
    ImeRequestError, ImeSurroundingText,
};

/// Events one window may hold between drains. Moves and wheel merge, so
/// this bounds bursts of transitions.
const ENDPOINT_EVENTS: usize = 1024;
const ENDPOINT_PAYLOAD_BYTES: usize = 1024 * 1024;

/// The process's clipboard: one system resource, opened once.
pub(super) fn process_clipboard() -> SharedClipboardHost {
    static CLIPBOARD: OnceLock<SharedClipboardHost> = OnceLock::new();
    CLIPBOARD.get_or_init(default_shared_clipboard).clone()
}

/// One window's input source.
#[derive(Debug)]
pub(super) struct WindowInputSource {
    pub(super) endpoint: InputEndpoint,
    pub(super) sequencer: InputSequencer,
    /// The cursor the Runtime last asked for.
    pub(super) runtime_cursor: nana_ui_platform::CursorIcon,
    /// Icon and visibility last set on the native window.
    pub(super) applied_cursor: Option<(CursorIcon, bool)>,
    /// Text-input state the Runtime last asked for.
    pub(super) ime: Option<TextInputContext>,
    /// The native IME session: the state last applied to the window.
    pub(super) applied_ime: Option<TextInputContext>,
    /// Focus changed since the last native apply. Keep this edge even if
    /// loss and regain arrive in the same turn with the same editor.
    ime_reset_pending: bool,
}

impl WindowInputSource {
    pub(super) fn new(source: InputSourceId, generation: EndpointGeneration) -> Self {
        Self {
            endpoint: InputEndpoint::new(ENDPOINT_EVENTS, ENDPOINT_PAYLOAD_BYTES),
            sequencer: InputSequencer::new(source, generation),
            runtime_cursor: nana_ui_platform::CursorIcon::Default,
            applied_cursor: None,
            ime: None,
            applied_ime: None,
            ime_reset_pending: false,
        }
    }

    /// Runtime cancels preedit on window focus loss. The native context
    /// must cancel it too, rather than resume invisible marked text on
    /// return. Defer the reset out of the platform's input callbacks.
    pub(super) fn reset_ime(&mut self) {
        self.ime_reset_pending = true;
    }

    /// What the native IME needs to catch up with the state the Runtime
    /// asked for, recorded as applied. `ImeApply::None` when it already has.
    pub(super) fn take_ime_apply(&mut self) -> ImeApply {
        let reset = std::mem::take(&mut self.ime_reset_pending);
        let (mut apply, session) = ime_apply(self.applied_ime.as_ref(), self.ime.as_ref());
        if reset
            && !matches!(apply, ImeApply::Replace { .. })
            && let Some(session) = &session
        {
            apply = ImeApply::Reset {
                capabilities: ime_capabilities(session),
                data: ime_request_data(session),
            };
        }
        self.applied_ime = session;
        apply
    }

    /// The pointer left: the window shows the default cursor, and so does
    /// the next composition. The Runtime forgot its cursor on leave and
    /// sends nothing for a default one on the next enter, so a cursor kept
    /// here (a text beam, `cursor: none`) would come back over plain UI.
    pub(super) fn reset_cursor(&mut self) -> (CursorIcon, bool) {
        self.runtime_cursor = nana_ui_platform::CursorIcon::Default;
        let applied = native_cursor(self.runtime_cursor);
        self.applied_cursor = Some(applied);
        applied
    }
}

/// The host services of one window for one drain.
pub(super) struct NativeWindowServices<'a> {
    pub(super) clipboard: &'a SharedClipboardHost,
    pub(super) runtime_cursor: &'a mut nana_ui_platform::CursorIcon,
    pub(super) ime: &'a mut Option<TextInputContext>,
    /// Set when the Runtime's cursor changed; the host composes and applies
    /// it once the drain is over.
    pub(super) cursor_changed: bool,
}

impl<'a> NativeWindowServices<'a> {
    /// Services over `source`'s slots, leaving its endpoint free to drain.
    pub(super) fn of(
        clipboard: &'a SharedClipboardHost,
        source: &'a mut WindowInputSource,
    ) -> (
        Self,
        &'a mut InputEndpoint,
        InputSourceId,
        EndpointGeneration,
    ) {
        let input_id = source.sequencer.source();
        let generation = source.sequencer.generation();
        (
            Self {
                clipboard,
                runtime_cursor: &mut source.runtime_cursor,
                ime: &mut source.ime,
                cursor_changed: false,
            },
            &mut source.endpoint,
            input_id,
            generation,
        )
    }
}

impl HostServices for NativeWindowServices<'_> {
    fn set_cursor(&mut self, cursor: nana_ui_platform::CursorIcon) {
        *self.runtime_cursor = cursor;
        self.cursor_changed = true;
    }

    fn set_text_input(&mut self, state: Option<&TextInputContext>) {
        // Follow the focused editable field, not NSWindow key status: gating
        // on focus disables the IME while its own candidate panel is key.
        // Recorded only: routing runs inside the platform's key and IME
        // messages, and an IME request made there re-enters the window
        // procedure (winit's Win32 backend then waits on its own window
        // lock). The host applies it once the turn's messages are handled.
        *self.ime = state.cloned();
    }

    fn read_clipboard(&mut self) -> Result<Option<String>, HostServiceError> {
        read_shared_clipboard(self.clipboard)
    }

    fn write_clipboard(&mut self, text: &str) -> Result<(), HostServiceError> {
        write_shared_clipboard(self.clipboard, text)
    }

    fn read_clipboard_token(&mut self) -> Result<Option<String>, HostServiceError> {
        read_shared_clipboard_token(self.clipboard)
    }

    fn write_clipboard_with_token(
        &mut self,
        text: &str,
        token: &str,
    ) -> Result<(), HostServiceError> {
        write_shared_clipboard_with_token(self.clipboard, text, token)
    }
}

/// The native icon for the Runtime's cursor, and whether it shows.
pub(super) fn native_cursor(cursor: nana_ui_platform::CursorIcon) -> (CursorIcon, bool) {
    use nana_ui_platform::CursorIcon as Runtime;
    match cursor {
        Runtime::Default => (CursorIcon::Default, true),
        Runtime::Pointer => (CursorIcon::Pointer, true),
        Runtime::Text => (CursorIcon::Text, true),
        Runtime::Move => (CursorIcon::Move, true),
        Runtime::Grab => (CursorIcon::Grab, true),
        Runtime::Grabbing => (CursorIcon::Grabbing, true),
        Runtime::NotAllowed => (CursorIcon::NotAllowed, true),
        Runtime::Crosshair => (CursorIcon::Crosshair, true),
        Runtime::Help => (CursorIcon::Help, true),
        Runtime::Wait => (CursorIcon::Wait, true),
        Runtime::Progress => (CursorIcon::Progress, true),
        Runtime::ZoomIn => (CursorIcon::ZoomIn, true),
        Runtime::ZoomOut => (CursorIcon::ZoomOut, true),
        Runtime::EwResize => (CursorIcon::EwResize, true),
        Runtime::NsResize => (CursorIcon::NsResize, true),
        Runtime::Hidden => (CursorIcon::Default, false),
    }
}

/// What to ask of the native IME to go from its session to the next state.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum ImeApply {
    None,
    Disable,
    Enable {
        capabilities: ImeCapabilities,
        data: ImeRequestData,
    },
    Replace {
        capabilities: ImeCapabilities,
        data: ImeRequestData,
    },
    Reset {
        capabilities: ImeCapabilities,
        data: ImeRequestData,
    },
    Update(ImeRequestData),
}

fn ime_capabilities(state: &TextInputContext) -> ImeCapabilities {
    let mut capabilities = ImeCapabilities::new().with_hint_and_purpose();
    if state.cursor_area.is_some() {
        capabilities = capabilities.with_cursor_area();
    }
    if state.surrounding.is_some() {
        capabilities = capabilities.with_surrounding_text();
    }
    capabilities
}

fn ime_request_data(state: &TextInputContext) -> ImeRequestData {
    let purpose = match state.purpose {
        TextInputPurpose::Normal => ImePurpose::Normal,
        TextInputPurpose::Password => ImePurpose::Password,
        TextInputPurpose::Terminal => ImePurpose::Terminal,
    };
    let mut data = ImeRequestData::default().with_hint_and_purpose(ImeHint::NONE, purpose);
    if let Some(cursor) = state.cursor_area {
        data = data.with_cursor_area(
            // The area the candidates must not cover, from its top-left
            // corner: the IME places its list beside or below it.
            winit::dpi::LogicalPosition::new(cursor.x, cursor.y).into(),
            winit::dpi::LogicalSize::new(cursor.width.max(1.0), cursor.height.max(1.0)).into(),
        );
    }
    if let Some(surrounding) = state.surrounding.as_ref().and_then(|surrounding| {
        ImeSurroundingText::new(
            surrounding.text.clone(),
            surrounding.cursor,
            surrounding.anchor,
        )
        .ok()
    }) {
        data = data.with_surrounding_text(surrounding);
    }
    data
}

pub(super) fn ime_apply(
    applied: Option<&TextInputContext>,
    next: Option<&TextInputContext>,
) -> (ImeApply, Option<TextInputContext>) {
    let Some(next) = next else {
        let apply = applied.map_or(ImeApply::None, |_| ImeApply::Disable);
        return (apply, None);
    };
    let same_editor =
        |applied: &TextInputContext| applied.owner == next.owner && applied.purpose == next.purpose;
    // The same editor keeps its session: tearing it down cancels the
    // composition being typed. A value the editor has no answer for this
    // once (a caret not laid out yet) keeps the one the session has.
    let session = match applied {
        Some(applied) if same_editor(applied) => TextInputContext {
            cursor_area: next.cursor_area.or(applied.cursor_area),
            surrounding: next
                .surrounding
                .clone()
                .or_else(|| applied.surrounding.clone()),
            ..next.clone()
        },
        _ => next.clone(),
    };
    let capabilities = ime_capabilities(&session);
    let apply = match applied {
        None => ImeApply::Enable {
            capabilities,
            data: ime_request_data(&session),
        },
        Some(applied) if *applied == session => ImeApply::None,
        // Another editor is a new session: whatever the IME was composing
        // belongs to the field focus left. A capability the session was
        // enabled without is declared only by an enable.
        Some(applied) if !same_editor(applied) || ime_capabilities(applied) != capabilities => {
            ImeApply::Replace {
                capabilities,
                data: ime_request_data(&session),
            }
        }
        Some(_) => ImeApply::Update(ime_request_data(&session)),
    };
    (apply, Some(session))
}

fn enable_ime(
    window: &dyn winit::window::Window,
    capabilities: ImeCapabilities,
    data: ImeRequestData,
) -> Result<(), String> {
    let Some(enable) = ImeEnableRequest::new(capabilities, data.clone()) else {
        return Err("IME capabilities do not match initial request data".into());
    };
    match window.request_ime_update(ImeRequest::Enable(enable)) {
        Err(ImeRequestError::AlreadyEnabled) => window.request_ime_update(ImeRequest::Update(data)),
        result => result,
    }
    .map_err(|error| error.to_string())
}

pub(super) fn apply_text_input(
    window: &dyn winit::window::Window,
    apply: ImeApply,
) -> Result<(), String> {
    match apply {
        ImeApply::None => Ok(()),
        ImeApply::Disable => {
            nana_window::discard_native_ime_preedit(window);
            window
                .request_ime_update(ImeRequest::Disable)
                .map_err(|error| error.to_string())
        }
        ImeApply::Enable { capabilities, data } => enable_ime(window, capabilities, data),
        ImeApply::Replace { capabilities, data } => {
            nana_window::discard_native_ime_preedit(window);
            window
                .request_ime_update(ImeRequest::Disable)
                .map_err(|error| error.to_string())?;
            enable_ime(window, capabilities, data)
        }
        ImeApply::Reset { capabilities, data } => {
            nana_window::discard_native_ime_preedit(window);
            // unmarkText cancels AppKit's composition without changing its
            // declared capabilities. Keep a matching session; other
            // platforms cancel through Disable.
            #[cfg(target_os = "macos")]
            let disable = window
                .ime_capabilities()
                .is_some_and(|active| active != capabilities);
            #[cfg(not(target_os = "macos"))]
            let disable = true;
            if disable {
                window
                    .request_ime_update(ImeRequest::Disable)
                    .map_err(|error| error.to_string())?;
            }
            enable_ime(window, capabilities, data)
        }
        ImeApply::Update(data) => window
            .request_ime_update(ImeRequest::Update(data))
            .map_err(|error| error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui_core::LogicalRect;
    use nana_ui_platform::SurroundingText;

    fn state(cursor_area: bool, surrounding: bool) -> TextInputContext {
        TextInputContext {
            owner: 1,
            purpose: TextInputPurpose::Normal,
            cursor_area: cursor_area.then(|| LogicalRect::new(10.0, 20.0, 2.0, 18.0)),
            surrounding: surrounding.then(|| SurroundingText {
                text: "abc".into(),
                cursor: 3,
                anchor: 3,
            }),
        }
    }

    #[test]
    fn the_ime_enables_once_then_updates_its_caret() {
        let first = state(true, true);
        assert!(matches!(
            ime_apply(None, Some(&first)).0,
            ImeApply::Enable { .. }
        ));
        let moved = TextInputContext {
            cursor_area: Some(LogicalRect::new(30.0, 20.0, 2.0, 18.0)),
            ..first.clone()
        };
        assert!(matches!(
            ime_apply(Some(&first), Some(&moved)).0,
            ImeApply::Update(_)
        ));
    }

    #[test]
    fn an_unchanged_state_asks_nothing_of_the_ime() {
        let first = state(true, true);
        assert_eq!(
            ime_apply(Some(&first), Some(&first)),
            (ImeApply::None, Some(first.clone()))
        );
    }

    #[test]
    fn a_caret_missing_for_a_moment_keeps_the_session() {
        // Typing into the same editor never tears the session down: that
        // cancels the composition, and on Windows the IME calls back into
        // the window while winit holds its state.
        let first = state(true, true);
        let (apply, session) = ime_apply(Some(&first), Some(&state(false, true)));
        assert_eq!(apply, ImeApply::None);
        assert_eq!(session, Some(first));
    }

    #[test]
    fn the_state_is_applied_at_the_end_of_the_turn() {
        let mut source = WindowInputSource::new(InputSourceId(1), EndpointGeneration(1));
        source.ime = Some(state(true, true));
        assert!(matches!(source.take_ime_apply(), ImeApply::Enable { .. }));
        assert_eq!(source.take_ime_apply(), ImeApply::None);
        source.ime = None;
        assert_eq!(source.take_ime_apply(), ImeApply::Disable);
        assert_eq!(source.take_ime_apply(), ImeApply::None);
    }

    #[test]
    fn another_field_replaces_the_ime_session() {
        let first = state(true, true);
        let other = TextInputContext {
            owner: 2,
            ..first.clone()
        };
        assert!(matches!(
            ime_apply(Some(&first), Some(&other)).0,
            ImeApply::Replace { .. }
        ));
    }

    #[test]
    fn the_candidate_area_is_the_caret_itself() {
        let data = ime_request_data(&state(true, false));
        assert_eq!(
            data.cursor_area,
            Some((
                winit::dpi::LogicalPosition::new(10.0, 20.0).into(),
                winit::dpi::LogicalSize::new(2.0, 18.0).into(),
            ))
        );
    }

    #[test]
    fn focus_changes_cancel_the_native_session_even_for_the_same_editor() {
        let mut source = WindowInputSource::new(InputSourceId(1), EndpointGeneration(1));
        source.ime = Some(state(true, true));
        assert!(matches!(source.take_ime_apply(), ImeApply::Enable { .. }));
        source.reset_ime();
        // Losing and regaining focus before about_to_wait must not erase
        // the cancellation, even though the requested editor is unchanged.
        source.reset_ime();
        assert!(matches!(source.take_ime_apply(), ImeApply::Reset { .. }));
        assert_eq!(source.take_ime_apply(), ImeApply::None);
    }

    #[test]
    fn a_focus_reset_without_an_editor_does_not_enable_ime() {
        let mut source = WindowInputSource::new(InputSourceId(1), EndpointGeneration(1));
        source.ime = Some(state(true, true));
        source.take_ime_apply();
        source.reset_ime();
        source.ime = None;
        assert_eq!(source.take_ime_apply(), ImeApply::Disable);
        assert_eq!(source.take_ime_apply(), ImeApply::None);
    }

    #[test]
    fn resetting_one_window_leaves_the_other_composition_session_alone() {
        let mut first = WindowInputSource::new(InputSourceId(1), EndpointGeneration(1));
        let mut other = WindowInputSource::new(InputSourceId(2), EndpointGeneration(1));
        for source in [&mut first, &mut other] {
            source.ime = Some(state(true, true));
            source.take_ime_apply();
        }
        first.reset_ime();
        assert!(matches!(first.take_ime_apply(), ImeApply::Reset { .. }));
        assert_eq!(other.take_ime_apply(), ImeApply::None);
    }

    #[test]
    fn a_focus_reset_still_replaces_a_changed_editor() {
        let mut source = WindowInputSource::new(InputSourceId(1), EndpointGeneration(1));
        source.ime = Some(state(true, true));
        source.take_ime_apply();
        source.reset_ime();
        source.ime.as_mut().unwrap().owner = 2;
        assert!(matches!(source.take_ime_apply(), ImeApply::Replace { .. }));
    }

    #[test]
    fn native_unmark_after_window_regain_does_not_start_an_empty_composition() {
        use nana_ui_platform::{InputModifiers, InputPayload, KeyInput, KeyState};
        use nana_ui_runtime::{AppContext, DocumentId, HeadlessInput, TextInput};

        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let editor = context
            .create_component(document, TextInput::new("ce"))
            .unwrap();
        context.focus_node(document, editor.stable_id()).unwrap();
        let mut input = HeadlessInput::bind(&mut context, document);
        input
            .route(&mut context, InputPayload::Focus { focused: true })
            .unwrap();
        input
            .composition(
                &mut context,
                super::super::platform_composition(&winit::event::Ime::Preedit(
                    "ha".into(),
                    Some((2, 2)),
                )),
            )
            .unwrap();
        input
            .route(&mut context, InputPayload::Focus { focused: false })
            .unwrap();
        input
            .route(&mut context, InputPayload::Focus { focused: true })
            .unwrap();
        // AppKit unmarkText queues this after the focus event. It cancels
        // the native conversion; it must not re-enter Runtime composition.
        input
            .composition(
                &mut context,
                super::super::platform_composition(&winit::event::Ime::Preedit(
                    String::new(),
                    None,
                )),
            )
            .unwrap();
        assert!(context.world().ime(editor.stable_id()).is_none());
        let select_all = KeyInput::named(
            "KeyA",
            "a",
            KeyState::Pressed,
            InputModifiers {
                meta: cfg!(target_os = "macos"),
                control: !cfg!(target_os = "macos"),
                ..InputModifiers::default()
            },
        );
        input
            .press(&mut context, select_all.clone(), Some("a"), None)
            .unwrap();
        // Cancel restores the original selection. Repeating Select All is
        // a no-op, but its native key text must still not insert an 'a'.
        input
            .press(&mut context, select_all, Some("a"), None)
            .unwrap();
        input.text(&mut context, "好").unwrap();
        assert_eq!(context.world().text(editor.stable_id()), Some("好"));
    }

    #[test]
    fn a_new_capability_replaces_the_ime_session() {
        assert!(matches!(
            ime_apply(Some(&state(false, true)), Some(&state(true, true))).0,
            ImeApply::Replace { .. }
        ));
    }

    #[test]
    fn leaving_the_field_disables_the_ime() {
        assert_eq!(
            ime_apply(Some(&state(true, true)), None),
            (ImeApply::Disable, None)
        );
        assert_eq!(ime_apply(None, None), (ImeApply::None, None));
    }

    #[test]
    fn a_pointer_leaving_resets_the_cursor_the_runtime_asked_for() {
        let mut source = WindowInputSource::new(InputSourceId(1), EndpointGeneration(1));
        source.runtime_cursor = nana_ui_platform::CursorIcon::Hidden;
        source.applied_cursor = Some(native_cursor(source.runtime_cursor));
        assert_eq!(source.reset_cursor(), (CursorIcon::Default, true));
        // Re-entering over plain UI: the Runtime sends nothing, and what the
        // host composes is the default.
        assert_eq!(
            native_cursor(source.runtime_cursor),
            (CursorIcon::Default, true)
        );
    }

    #[test]
    fn hidden_runtime_cursor_hides_the_native_one() {
        assert_eq!(
            native_cursor(nana_ui_platform::CursorIcon::Hidden),
            (CursorIcon::Default, false)
        );
        assert_eq!(
            native_cursor(nana_ui_platform::CursorIcon::EwResize),
            (CursorIcon::EwResize, true)
        );
    }
}
