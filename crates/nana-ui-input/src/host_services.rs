//! What the Runtime asks of the host while it routes input.
//!
//! Two of these are state, not requests: the cursor the Runtime wants under
//! the pointer and the text-input (IME) state of the focused editor. The
//! router calls [`HostServices::set_cursor`] and
//! [`HostServices::set_text_input`] only when that state changes, so a host
//! never sees a queue of stale intents and a full queue can never hold input
//! back. The clipboard is the one request with an answer; hosts answer it
//! synchronously on the event-loop thread and report a busy backend rather
//! than waiting on it.

use nana_ui_core::LogicalRect;

/// The cursor the Runtime wants under the pointer. Hosts compose it with
/// their own (a window frame edge, a program override) before showing it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum CursorIcon {
    #[default]
    Default,
    Pointer,
    Text,
    Move,
    Grab,
    Grabbing,
    NotAllowed,
    Crosshair,
    Help,
    Wait,
    Progress,
    ZoomIn,
    ZoomOut,
    /// A horizontal resize handle (a split or dock bar between columns).
    EwResize,
    /// A vertical resize handle (a bar between rows).
    NsResize,
    /// `cursor: none`.
    Hidden,
}

/// What kind of text the focused editor takes, for the IME and soft keyboard.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum TextInputPurpose {
    #[default]
    Normal,
    /// Secure entry: no surrounding text, no suggestions.
    Password,
    /// A terminal: keys go to the program, composition to its input line.
    Terminal,
}

/// Text around the caret that the IME may read, as a window of the field.
///
/// Never more than [`SurroundingText::MAX_BYTES`] of it, whatever the size of
/// the field; offsets are UTF-8 byte offsets into `text` on character
/// boundaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurroundingText {
    pub text: String,
    pub cursor: usize,
    pub anchor: usize,
}

impl SurroundingText {
    /// The most surrounding text a host is handed (winit's own limit).
    pub const MAX_BYTES: usize = 4000;
}

/// The text-input state of the focused editor. The host enables its IME for
/// `Some` and disables it for `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct TextInputContext {
    pub purpose: TextInputPurpose,
    /// Caret, or the field when it has no caret geometry, in the window's
    /// logical coordinates. The IME candidate window anchors below it.
    pub cursor_area: Option<LogicalRect>,
    /// `None` for secure entry and for terminals.
    pub surrounding: Option<SurroundingText>,
}

/// Why a clipboard call returned nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostServiceError {
    /// This host has no clipboard.
    Unsupported,
    /// The backend is in use; the host answers instead of waiting for it.
    Busy,
    /// The backend refused or failed.
    Failed,
}

/// The host side of input routing. One per input source (a window, a
/// headless session), passed to every route and drain of that source.
pub trait HostServices {
    /// The cursor the Runtime wants now. Called only when it changes.
    fn set_cursor(&mut self, cursor: CursorIcon);

    /// The focused editor's text-input state; `None` disables text input.
    /// Called when it changes and again when the source regains focus.
    fn set_text_input(&mut self, state: Option<&TextInputContext>);

    /// Plain text on the clipboard, `Ok(None)` when it holds none.
    fn read_clipboard(&mut self) -> Result<Option<String>, HostServiceError>;

    fn write_clipboard(&mut self, text: &str) -> Result<(), HostServiceError>;
}

/// A host with none of these services: the cursor and text input are
/// ignored and the clipboard is unsupported.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnsupportedHostServices;

impl HostServices for UnsupportedHostServices {
    fn set_cursor(&mut self, _cursor: CursorIcon) {}

    fn set_text_input(&mut self, _state: Option<&TextInputContext>) {}

    fn read_clipboard(&mut self) -> Result<Option<String>, HostServiceError> {
        Err(HostServiceError::Unsupported)
    }

    fn write_clipboard(&mut self, _text: &str) -> Result<(), HostServiceError> {
        Err(HostServiceError::Unsupported)
    }
}

/// How often a headless host was asked for each service.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HostServiceCounters {
    pub cursor_updates: u64,
    pub text_input_updates: u64,
    pub clipboard_reads: u64,
    pub clipboard_writes: u64,
}

/// Services for a host without a window: devtools sessions, tests, offscreen
/// harnesses. It keeps what it was told, so a fixture can read the cursor and
/// IME state a window would have shown, and holds a private clipboard.
#[derive(Debug, Default, Clone)]
pub struct HeadlessHostServices {
    cursor: CursorIcon,
    text_input: Option<TextInputContext>,
    clipboard: Option<String>,
    counters: HostServiceCounters,
}

impl HeadlessHostServices {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cursor(&self) -> CursorIcon {
        self.cursor
    }

    pub fn text_input(&self) -> Option<&TextInputContext> {
        self.text_input.as_ref()
    }

    pub fn clipboard(&self) -> Option<&str> {
        self.clipboard.as_deref()
    }

    pub fn set_clipboard(&mut self, text: Option<String>) {
        self.clipboard = text;
    }

    pub fn counters(&self) -> HostServiceCounters {
        self.counters
    }
}

impl HostServices for HeadlessHostServices {
    fn set_cursor(&mut self, cursor: CursorIcon) {
        self.counters.cursor_updates += 1;
        self.cursor = cursor;
    }

    fn set_text_input(&mut self, state: Option<&TextInputContext>) {
        self.counters.text_input_updates += 1;
        self.text_input = state.cloned();
    }

    fn read_clipboard(&mut self) -> Result<Option<String>, HostServiceError> {
        self.counters.clipboard_reads += 1;
        Ok(self.clipboard.clone())
    }

    fn write_clipboard(&mut self, text: &str) -> Result<(), HostServiceError> {
        self.counters.clipboard_writes += 1;
        self.clipboard = Some(text.to_owned());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headless_services_keep_what_the_router_last_sent() {
        let mut services = HeadlessHostServices::new();
        services.set_cursor(CursorIcon::Text);
        let state = TextInputContext {
            purpose: TextInputPurpose::Normal,
            cursor_area: None,
            surrounding: Some(SurroundingText {
                text: "abc".into(),
                cursor: 3,
                anchor: 1,
            }),
        };
        services.set_text_input(Some(&state));
        services.write_clipboard("copied").unwrap();
        assert_eq!(services.cursor(), CursorIcon::Text);
        assert_eq!(services.text_input(), Some(&state));
        assert_eq!(services.read_clipboard(), Ok(Some("copied".into())));
        services.set_text_input(None);
        assert_eq!(services.text_input(), None);
        assert_eq!(
            services.counters(),
            HostServiceCounters {
                cursor_updates: 1,
                text_input_updates: 2,
                clipboard_reads: 1,
                clipboard_writes: 1,
            }
        );
    }

    #[test]
    fn unsupported_services_say_so() {
        let mut services = UnsupportedHostServices;
        assert_eq!(
            services.read_clipboard(),
            Err(HostServiceError::Unsupported)
        );
        assert_eq!(
            services.write_clipboard("x"),
            Err(HostServiceError::Unsupported)
        );
    }
}
