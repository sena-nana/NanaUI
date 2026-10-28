//! Clipboard host boundary.
//!
//! Desktop uses the OS clipboard via `arboard` ([`OsClipboard`]). Android does
//! not compile arboard; [`default_shared_clipboard`] installs
//! [`AndroidClipboard`] (JNI `ClipboardManager`). OHOS does not compile it
//! either and has no default backend: its host answers the clipboard through
//! its own `HostServices`, and the default reports it unsupported
//! (`docs/reference/ohos.md`).

use std::sync::{Arc, Mutex};

/// Minimal clipboard contract for Nana hosts — never System WebView.
pub trait ClipboardHost: Send {
    /// Whether this host currently has a usable clipboard capability.
    /// Implementations may keep the default when failures are reported by
    /// their read/write methods instead.
    fn is_available(&self) -> bool {
        true
    }
    fn read_text(&mut self) -> Option<String>;
    fn write_text(&mut self, text: &str) -> bool;
    /// A count the platform advances with every write to the clipboard, by
    /// any process. Read right after a write, it later tells whether the
    /// clipboard still holds that write. `None` where the platform keeps
    /// none.
    fn change_count(&mut self) -> Option<u64> {
        None
    }
}

/// Shared clipboard handle installed into host API registries.
pub type SharedClipboardHost = Arc<Mutex<Box<dyn ClipboardHost>>>;

/// Wrap any [`ClipboardHost`] for host-op registration.
pub fn shared_clipboard<C: ClipboardHost + 'static>(clipboard: C) -> SharedClipboardHost {
    Arc::new(Mutex::new(Box::new(clipboard)))
}

/// Platform default: OS clipboard on desktop; JNI ClipboardManager on Android;
/// [`UnsupportedClipboard`] on OHOS and iOS, whose hosts inject the system
/// pasteboard.
pub fn default_shared_clipboard() -> SharedClipboardHost {
    #[cfg(target_os = "android")]
    {
        shared_clipboard(AndroidClipboard)
    }
    #[cfg(any(target_os = "ios", target_env = "ohos"))]
    {
        shared_clipboard(UnsupportedClipboard)
    }
    #[cfg(not(any(target_os = "android", target_os = "ios", target_env = "ohos")))]
    {
        shared_clipboard(OsClipboard::new())
    }
}

/// Always-unavailable clipboard (tests / hosts that opt out).
#[derive(Debug, Default, Clone, Copy)]
pub struct UnsupportedClipboard;

impl ClipboardHost for UnsupportedClipboard {
    fn is_available(&self) -> bool {
        false
    }

    fn read_text(&mut self) -> Option<String> {
        None
    }

    fn write_text(&mut self, _text: &str) -> bool {
        false
    }
}

/// In-process clipboard for unit tests (does not touch the OS pasteboard).
#[derive(Debug, Default, Clone)]
pub struct MemoryClipboard {
    text: Option<String>,
    changes: u64,
}

impl MemoryClipboard {
    pub fn new() -> Self {
        Self::default()
    }
}

impl ClipboardHost for MemoryClipboard {
    fn read_text(&mut self) -> Option<String> {
        self.text.clone()
    }

    fn write_text(&mut self, text: &str) -> bool {
        self.text = Some(text.to_owned());
        self.changes += 1;
        true
    }

    fn change_count(&mut self) -> Option<u64> {
        Some(self.changes)
    }
}

/// Desktop OS clipboard (`arboard`). Construction never panics; unavailable
/// backends report failed reads/writes. Not compiled on Android, iOS or OHOS.
#[cfg(not(any(target_os = "android", target_os = "ios", target_env = "ohos")))]
pub struct OsClipboard {
    inner: Option<arboard::Clipboard>,
}

#[cfg(not(any(target_os = "android", target_os = "ios", target_env = "ohos")))]
impl std::fmt::Debug for OsClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OsClipboard")
            .field("available", &self.inner.is_some())
            .finish()
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios", target_env = "ohos")))]
impl OsClipboard {
    pub fn new() -> Self {
        Self {
            inner: arboard::Clipboard::new().ok(),
        }
    }

    pub fn is_available(&self) -> bool {
        self.inner.is_some()
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios", target_env = "ohos")))]
impl Default for OsClipboard {
    fn default() -> Self {
        Self::new()
    }
}

/// Android system clipboard via `ClipboardManager`.
///
/// Reads/writes fail honestly when the JNI Activity context is missing (unit
/// tests, or a call before `android_main`). This is not a second pasteboard.
#[cfg(target_os = "android")]
#[derive(Debug, Default, Clone, Copy)]
pub struct AndroidClipboard;

#[cfg(target_os = "android")]
impl ClipboardHost for AndroidClipboard {
    fn read_text(&mut self) -> Option<String> {
        android_clipboard_text(None)
    }

    fn write_text(&mut self, text: &str) -> bool {
        android_clipboard_text(Some(text)).is_some()
    }
}

#[cfg(target_os = "android")]
fn android_clipboard_text(write: Option<&str>) -> Option<String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        android_clipboard_text_inner(write)
    }))
    .ok()
    .flatten()
}

#[cfg(target_os = "android")]
fn android_clipboard_text_inner(write: Option<&str>) -> Option<String> {
    use std::mem::ManuallyDrop;

    use jni::JavaVM;
    use jni::objects::{JObject, JString, JValue};

    let ctx = ndk_context::android_context();
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }.ok()?;
    let mut env = vm.attach_current_thread().ok()?;
    let context =
        ManuallyDrop::new(unsafe { JObject::from_raw(ctx.context() as jni::sys::jobject) });
    let service = env.new_string("clipboard").ok()?;
    let manager = env
        .call_method(
            &*context,
            "getSystemService",
            "(Ljava/lang/String;)Ljava/lang/Object;",
            &[JValue::Object(&service)],
        )
        .ok()?
        .l()
        .ok()?;
    if manager.is_null() {
        return None;
    }
    match write {
        Some(text) => {
            let label = env.new_string("nana").ok()?;
            let value = env.new_string(text).ok()?;
            let clip_class = env.find_class("android/content/ClipData").ok()?;
            let clip = env
                .call_static_method(
                    clip_class,
                    "newPlainText",
                    "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;",
                    &[JValue::Object(&label), JValue::Object(&value)],
                )
                .ok()?
                .l()
                .ok()?;
            env.call_method(
                &manager,
                "setPrimaryClip",
                "(Landroid/content/ClipData;)V",
                &[JValue::Object(&clip)],
            )
            .ok()?;
            Some(text.to_owned())
        }
        None => {
            let clip = env
                .call_method(
                    &manager,
                    "getPrimaryClip",
                    "()Landroid/content/ClipData;",
                    &[],
                )
                .ok()?
                .l()
                .ok()?;
            if clip.is_null() {
                return None;
            }
            let count = env
                .call_method(&clip, "getItemCount", "()I", &[])
                .ok()?
                .i()
                .ok()?;
            if count <= 0 {
                return None;
            }
            let item = env
                .call_method(
                    &clip,
                    "getItemAt",
                    "(I)Landroid/content/ClipData$Item;",
                    &[JValue::Int(0)],
                )
                .ok()?
                .l()
                .ok()?;
            let sequence = env
                .call_method(
                    &item,
                    "coerceToText",
                    "(Landroid/content/Context;)Ljava/lang/CharSequence;",
                    &[JValue::Object(&*context)],
                )
                .ok()?
                .l()
                .ok()?;
            if sequence.is_null() {
                return None;
            }
            let jstr = env
                .call_method(&sequence, "toString", "()Ljava/lang/String;", &[])
                .ok()?
                .l()
                .ok()?;
            let jstr = JString::from(jstr);
            env.get_string(&jstr).ok().map(|s| s.into())
        }
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios", target_env = "ohos")))]
impl ClipboardHost for OsClipboard {
    fn is_available(&self) -> bool {
        self.inner.is_some()
    }

    fn read_text(&mut self) -> Option<String> {
        self.inner.as_mut()?.get_text().ok()
    }

    fn write_text(&mut self, text: &str) -> bool {
        match self.inner.as_mut() {
            Some(clipboard) => clipboard.set_text(text.to_owned()).is_ok(),
            None => false,
        }
    }

    fn change_count(&mut self) -> Option<u64> {
        self.inner.as_ref()?;
        os_clipboard_change_count()
    }
}

/// `NSPasteboard.changeCount` of the general pasteboard: it advances each
/// time an owner clears it to write.
#[cfg(target_os = "macos")]
fn os_clipboard_change_count() -> Option<u64> {
    let count = objc2_app_kit::NSPasteboard::generalPasteboard().changeCount();
    u64::try_from(count).ok()
}

/// `GetClipboardSequenceNumber`: it advances with every change of the
/// window station's clipboard. Zero means this process cannot see it.
#[cfg(windows)]
fn os_clipboard_change_count() -> Option<u64> {
    // SAFETY: no arguments and no preconditions; it reads a counter.
    let count = unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() };
    (count != 0).then_some(u64::from(count))
}

/// X11 and Wayland keep no change count the clipboard can be asked for.
#[cfg(not(any(
    target_os = "macos",
    windows,
    target_os = "android",
    target_os = "ios",
    target_env = "ohos"
)))]
fn os_clipboard_change_count() -> Option<u64> {
    None
}

/// Clipboard calls against a shared backend. The lock is tried, never
/// waited on: an OS or JNI clipboard call must not stall input routing.
pub fn read_shared_clipboard(
    clipboard: &SharedClipboardHost,
) -> Result<Option<String>, nana_ui_input::HostServiceError> {
    let mut clipboard = clipboard
        .try_lock()
        .map_err(|_| nana_ui_input::HostServiceError::Busy)?;
    if !clipboard.is_available() {
        return Err(nana_ui_input::HostServiceError::Unsupported);
    }
    Ok(clipboard.read_text())
}

/// The shared backend's [`ClipboardHost::change_count`]. See
/// [`read_shared_clipboard`].
pub fn shared_clipboard_change_count(
    clipboard: &SharedClipboardHost,
) -> Result<u64, nana_ui_input::HostServiceError> {
    let mut clipboard = clipboard
        .try_lock()
        .map_err(|_| nana_ui_input::HostServiceError::Busy)?;
    if !clipboard.is_available() {
        return Err(nana_ui_input::HostServiceError::Unsupported);
    }
    clipboard
        .change_count()
        .ok_or(nana_ui_input::HostServiceError::Unsupported)
}

/// See [`read_shared_clipboard`].
pub fn write_shared_clipboard(
    clipboard: &SharedClipboardHost,
    text: &str,
) -> Result<(), nana_ui_input::HostServiceError> {
    let mut clipboard = clipboard
        .try_lock()
        .map_err(|_| nana_ui_input::HostServiceError::Busy)?;
    if !clipboard.is_available() {
        return Err(nana_ui_input::HostServiceError::Unsupported);
    }
    if clipboard.write_text(text) {
        Ok(())
    } else {
        Err(nana_ui_input::HostServiceError::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_clipboard_is_inert() {
        let mut clip = UnsupportedClipboard;
        assert!(clip.read_text().is_none());
        assert!(!clip.write_text("nana"));
    }

    #[test]
    fn memory_clipboard_roundtrip() {
        let mut clip = MemoryClipboard::new();
        assert!(clip.read_text().is_none());
        assert!(clip.write_text("hello"));
        assert_eq!(clip.read_text().as_deref(), Some("hello"));
        assert!(clip.write_text(""));
        assert_eq!(clip.read_text().as_deref(), Some(""));
    }

    #[test]
    #[cfg(target_os = "android")]
    fn android_default_clipboard_uses_jni_backend() {
        let host = default_shared_clipboard();
        let mut clip = host.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        // Host `cargo test` has no Activity context; JNI must fail closed
        // rather than report a successful pasteboard round-trip.
        assert!(clip.read_text().is_none());
        assert!(!clip.write_text("nana"));
    }

    #[test]
    #[cfg(not(any(target_os = "android", target_os = "ios", target_env = "ohos")))]
    fn os_clipboard_roundtrip_when_available() {
        let mut clip = OsClipboard::new();
        if !clip.is_available() {
            // Headless / locked-down environments may lack a pasteboard.
            return;
        }
        let marker = format!("nana-ui-clipboard-{}-{}", std::process::id(), "probe");
        let previous = clip.read_text();

        // The pasteboard is one machine-wide resource, so anything else on the
        // machine — including a second `cargo test` — can overwrite it between
        // our write and our read. That is interference, not a broken clipboard,
        // so the write/read pair is retried as a unit and only a run that never
        // round-trips is a failure. Asserting on a single attempt made this
        // test fail whenever two test processes overlapped.
        let mut observed = None;
        for _ in 0..8 {
            assert!(
                clip.write_text(&marker),
                "desktop OS clipboard write must succeed when available"
            );
            observed = clip.read_text();
            if observed.as_deref() == Some(marker.as_str()) {
                break;
            }
        }
        assert_eq!(
            observed.as_deref(),
            Some(marker.as_str()),
            "desktop OS clipboard read must return the written text"
        );

        // A write advances the change count and a read leaves it, retried
        // as a unit for the same reason as the round trip.
        #[cfg(any(target_os = "macos", windows))]
        {
            let mut stamped = false;
            for _ in 0..8 {
                let before = clip
                    .change_count()
                    .expect("the platform keeps a change count");
                assert!(clip.write_text(&marker));
                let written = clip.change_count();
                let _ = clip.read_text();
                stamped = written != Some(before) && clip.change_count() == written;
                if stamped {
                    break;
                }
            }
            assert!(stamped, "a write advances the count and a read does not");
        }

        if let Some(prev) = previous {
            let _ = clip.write_text(&prev);
        }
    }

    #[test]
    fn an_uninjected_clipboard_answers_unsupported() {
        let clipboard = shared_clipboard(UnsupportedClipboard);
        assert_eq!(
            read_shared_clipboard(&clipboard),
            Err(nana_ui_input::HostServiceError::Unsupported)
        );
        assert_eq!(
            write_shared_clipboard(&clipboard, "nana"),
            Err(nana_ui_input::HostServiceError::Unsupported)
        );
    }

    #[test]
    #[cfg(target_env = "ohos")]
    fn ohos_default_clipboard_is_unsupported_until_the_host_injects_one() {
        let clipboard = default_shared_clipboard();
        assert_eq!(
            read_shared_clipboard(&clipboard),
            Err(nana_ui_input::HostServiceError::Unsupported)
        );
    }

    #[test]
    fn a_write_advances_the_change_count_and_a_read_does_not() {
        let clipboard = shared_clipboard(MemoryClipboard::new());
        let before = shared_clipboard_change_count(&clipboard).unwrap();
        write_shared_clipboard(&clipboard, "same").unwrap();
        let written = shared_clipboard_change_count(&clipboard).unwrap();
        assert_ne!(written, before);
        read_shared_clipboard(&clipboard).unwrap();
        assert_eq!(shared_clipboard_change_count(&clipboard), Ok(written));
        write_shared_clipboard(&clipboard, "same").unwrap();
        assert_ne!(
            shared_clipboard_change_count(&clipboard),
            Ok(written),
            "the same text written again is another write"
        );
        assert_eq!(
            shared_clipboard_change_count(&shared_clipboard(UnsupportedClipboard)),
            Err(nana_ui_input::HostServiceError::Unsupported)
        );
    }

    #[test]
    fn a_busy_shared_clipboard_answers_busy_instead_of_waiting() {
        let clipboard = shared_clipboard(MemoryClipboard::new());
        write_shared_clipboard(&clipboard, "hello").unwrap();
        assert_eq!(read_shared_clipboard(&clipboard), Ok(Some("hello".into())));
        let _held = clipboard.lock().unwrap();
        assert_eq!(
            read_shared_clipboard(&clipboard),
            Err(nana_ui_input::HostServiceError::Busy)
        );
    }
}
