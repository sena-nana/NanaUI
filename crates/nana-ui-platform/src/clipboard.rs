//! Clipboard host boundary.
//!
//! Desktop uses the OS clipboard via `arboard` ([`OsClipboard`]). Android does
//! not compile arboard; [`default_shared_clipboard`] installs
//! [`AndroidClipboard`] (JNI `ClipboardManager`).

use std::sync::{Arc, Mutex};

/// Minimal clipboard contract for Nana hosts — never System WebView.
pub trait ClipboardHost: Send {
    fn read_text(&mut self) -> Option<String>;
    fn write_text(&mut self, text: &str) -> bool;
}

/// Shared clipboard handle installed into host API registries.
pub type SharedClipboardHost = Arc<Mutex<Box<dyn ClipboardHost>>>;

/// Wrap any [`ClipboardHost`] for host-op registration.
pub fn shared_clipboard<C: ClipboardHost + 'static>(clipboard: C) -> SharedClipboardHost {
    Arc::new(Mutex::new(Box::new(clipboard)))
}

/// Platform default: OS clipboard on desktop; JNI ClipboardManager on Android.
pub fn default_shared_clipboard() -> SharedClipboardHost {
    #[cfg(target_os = "android")]
    {
        shared_clipboard(AndroidClipboard)
    }
    #[cfg(not(target_os = "android"))]
    {
        shared_clipboard(OsClipboard::new())
    }
}

/// Always-unavailable clipboard (tests / hosts that opt out).
#[derive(Debug, Default, Clone, Copy)]
pub struct UnsupportedClipboard;

impl ClipboardHost for UnsupportedClipboard {
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
        true
    }
}

/// Desktop OS clipboard (`arboard`). Construction never panics; unavailable
/// backends report failed reads/writes. Not compiled on Android.
#[cfg(not(target_os = "android"))]
pub struct OsClipboard {
    inner: Option<arboard::Clipboard>,
}

#[cfg(not(target_os = "android"))]
impl std::fmt::Debug for OsClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OsClipboard")
            .field("available", &self.inner.is_some())
            .finish()
    }
}

#[cfg(not(target_os = "android"))]
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

#[cfg(not(target_os = "android"))]
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

#[cfg(not(target_os = "android"))]
impl ClipboardHost for OsClipboard {
    fn read_text(&mut self) -> Option<String> {
        self.inner.as_mut()?.get_text().ok()
    }

    fn write_text(&mut self, text: &str) -> bool {
        match self.inner.as_mut() {
            Some(clipboard) => clipboard.set_text(text.to_owned()).is_ok(),
            None => false,
        }
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
    #[cfg(not(target_os = "android"))]
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

        if let Some(prev) = previous {
            let _ = clip.write_text(&prev);
        }
    }
}
