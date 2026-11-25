//! What the Runtime asks of the host while it routes input.
//!
//! Two of these are state, not requests: the cursor the Runtime wants under
//! the pointer and the text-input (IME) state of the focused editor. The
//! router calls [`HostServices::set_cursor`] and
//! [`HostServices::set_text_input`] only when that state changes, so a host
//! never sees a queue of stale intents and a full queue can never hold input
//! back. The clipboard is the one request with an answer; hosts answer it
//! synchronously on the event-loop thread and report a busy backend rather
//! than waiting on it. Its change count tells a paste whether the clipboard
//! still holds what this process copied, so styling kept in-process pastes
//! back only onto that copy.

use nana_ui_core::{DataFormat, DataTransfer, LogicalRect, TransferImage, TransferOrigin};

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
    /// Which editor this is. Focus moving to another field (or terminal)
    /// changes it, and the host starts a new IME session: a composition in
    /// the old field must not commit into the new one.
    pub owner: u64,
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

    /// Full multi-format clipboard payload, `Ok(None)` when empty.
    fn read_clipboard_transfer(&mut self) -> Result<Option<DataTransfer>, HostServiceError> {
        let Some(text) = self.read_clipboard()? else {
            return Ok(None);
        };
        let mut transfer = DataTransfer::new(TransferOrigin::External);
        transfer
            .set_text(text)
            .map_err(|_| HostServiceError::Failed)?;
        Ok(Some(transfer))
    }

    /// Write a multi-format [`DataTransfer`] to the clipboard.
    ///
    /// Hosts that only implement plain text fail closed (`Unsupported`) when
    /// asked to write non-text formats with no `text/plain` representation.
    fn write_clipboard_transfer(
        &mut self,
        transfer: &DataTransfer,
    ) -> Result<(), HostServiceError> {
        transfer
            .lifetime()
            .check()
            .map_err(|_| HostServiceError::Failed)?;
        if transfer.has_format(&DataFormat::TextPlain) {
            let text = transfer
                .read_text()
                .map_err(|_| HostServiceError::Failed)?
                .ok_or(HostServiceError::Failed)?;
            return self.write_clipboard(&text);
        }
        Err(HostServiceError::Unsupported)
    }

    /// Convenience wrapper reading `image/rgba8` from [`Self::read_clipboard_transfer`].
    fn read_clipboard_image(&mut self) -> Result<Option<TransferImage>, HostServiceError> {
        let Some(transfer) = self.read_clipboard_transfer()? else {
            return Ok(None);
        };
        if !transfer.has_format(&DataFormat::ImageRgba8) {
            return Ok(None);
        }
        transfer
            .read_rgba_image()
            .map_err(|_| HostServiceError::Failed)
    }

    /// Convenience wrapper writing `image/rgba8` through [`Self::write_clipboard_transfer`].
    fn write_clipboard_image(&mut self, image: &TransferImage) -> Result<(), HostServiceError> {
        let mut transfer = DataTransfer::internal();
        transfer
            .set_rgba_image(image.clone())
            .map_err(|_| HostServiceError::Failed)?;
        self.write_clipboard_transfer(&transfer)
    }

    /// A count that advances with every write to the clipboard, by this
    /// process or any other, and not with reads. Read right after a write,
    /// an equal count at a later paste means the clipboard still holds that
    /// write (macOS `NSPasteboard.changeCount`, Windows
    /// `GetClipboardSequenceNumber`). `Unsupported` where the platform keeps
    /// none; callers then match on the text alone.
    fn clipboard_change_count(&mut self) -> Result<u64, HostServiceError> {
        Err(HostServiceError::Unsupported)
    }
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

    fn read_clipboard_transfer(&mut self) -> Result<Option<DataTransfer>, HostServiceError> {
        Err(HostServiceError::Unsupported)
    }

    fn write_clipboard_transfer(
        &mut self,
        _transfer: &DataTransfer,
    ) -> Result<(), HostServiceError> {
        Err(HostServiceError::Unsupported)
    }
}

/// Services for a host without a window: devtools sessions, tests, offscreen
/// harnesses. It keeps what it was told, so a fixture can read the cursor and
/// IME state a window would have shown, and holds a private multi-format clipboard.
#[derive(Debug, Default, Clone)]
pub struct HeadlessHostServices {
    cursor: CursorIcon,
    text_input: Option<TextInputContext>,
    clipboard: Option<DataTransfer>,
    clipboard_changes: u64,
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
        self.clipboard.as_ref().and_then(DataTransfer::eager_text)
    }

    pub fn clipboard_transfer(&self) -> Option<&DataTransfer> {
        self.clipboard.as_ref()
    }

    /// Put `text` on the clipboard as another program would: a write the
    /// change count sees.
    pub fn set_clipboard(&mut self, text: Option<String>) {
        self.clipboard = text.map(DataTransfer::from_text);
        self.clipboard_changes += 1;
    }

    /// Put a multi-format transfer on the clipboard as another program would.
    pub fn set_clipboard_transfer(&mut self, transfer: Option<DataTransfer>) {
        self.clipboard = transfer;
        self.clipboard_changes += 1;
    }
}

impl HostServices for HeadlessHostServices {
    fn set_cursor(&mut self, cursor: CursorIcon) {
        self.cursor = cursor;
    }

    fn set_text_input(&mut self, state: Option<&TextInputContext>) {
        self.text_input = state.cloned();
    }

    fn read_clipboard(&mut self) -> Result<Option<String>, HostServiceError> {
        let Some(transfer) = self.read_clipboard_transfer()? else {
            return Ok(None);
        };
        if !transfer.has_format(&DataFormat::TextPlain) {
            return Ok(None);
        }
        transfer.read_text().map_err(|_| HostServiceError::Failed)
    }

    fn write_clipboard(&mut self, text: &str) -> Result<(), HostServiceError> {
        self.write_clipboard_transfer(&DataTransfer::from_text(text))
    }

    fn read_clipboard_transfer(&mut self) -> Result<Option<DataTransfer>, HostServiceError> {
        Ok(self.clipboard.clone())
    }

    fn write_clipboard_transfer(
        &mut self,
        transfer: &DataTransfer,
    ) -> Result<(), HostServiceError> {
        transfer
            .lifetime()
            .check()
            .map_err(|_| HostServiceError::Failed)?;
        if transfer.is_empty() {
            return Err(HostServiceError::Failed);
        }
        self.clipboard = Some(transfer.clone());
        self.clipboard_changes += 1;
        Ok(())
    }

    fn clipboard_change_count(&mut self) -> Result<u64, HostServiceError> {
        Ok(self.clipboard_changes)
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
            owner: 1,
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
        assert_eq!(
            services.clipboard_change_count(),
            Err(HostServiceError::Unsupported)
        );
    }

    #[test]
    fn headless_clipboard_writes_advance_the_change_count() {
        let mut services = HeadlessHostServices::new();
        services.write_clipboard("same").unwrap();
        let copied = services.clipboard_change_count().unwrap();
        services.read_clipboard().unwrap();
        assert_eq!(services.clipboard_change_count(), Ok(copied));
        services.set_clipboard(Some("same".into()));
        assert_ne!(services.clipboard_change_count(), Ok(copied));
    }

    #[test]
    fn headless_supports_multiformat_image_and_lazy_clipboard_transfers() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        use nana_ui_core::{TransferCachePolicy, TransferError};

        let mut services = HeadlessHostServices::new();
        let mut transfer = DataTransfer::internal();
        transfer.set_text("plain").unwrap();
        transfer.set_html("<b>rich</b>").unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_ref = Arc::clone(&calls);
        let custom = DataFormat::custom("application/x-nana-item").unwrap();
        transfer.set_lazy_fn(
            custom.clone(),
            Some(3),
            TransferCachePolicy::Cache,
            move |_| {
                calls_ref.fetch_add(1, Ordering::Relaxed);
                Ok(vec![1, 2, 3])
            },
        );
        services.write_clipboard_transfer(&transfer).unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 0);

        let read = services.read_clipboard_transfer().unwrap().unwrap();
        assert_eq!(read.read_text().unwrap().as_deref(), Some("plain"));
        assert_eq!(read.read_html().unwrap().as_deref(), Some("<b>rich</b>"));
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        assert_eq!(
            read.read_bytes(&custom).unwrap().as_deref(),
            Some([1u8, 2, 3].as_slice())
        );
        assert_eq!(calls.load(Ordering::Relaxed), 1);

        let image = TransferImage::new(1, 1, [1, 2, 3, 255].as_slice()).unwrap();
        services.write_clipboard_image(&image).unwrap();
        assert_eq!(services.read_clipboard(), Ok(None));
        assert_eq!(services.read_clipboard_image(), Ok(Some(image.clone())));

        transfer.lifetime().cancel();
        assert_eq!(
            services.write_clipboard_transfer(&transfer),
            Err(HostServiceError::Failed)
        );
        let mut unsupported = UnsupportedHostServices;
        assert!(matches!(
            unsupported.read_clipboard_transfer(),
            Err(HostServiceError::Unsupported)
        ));
        assert_eq!(
            unsupported.write_clipboard_image(&image),
            Err(HostServiceError::Unsupported)
        );
        let _ = TransferError::Cancelled;
    }
}
