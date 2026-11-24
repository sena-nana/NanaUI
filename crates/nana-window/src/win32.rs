//! Small Win32 helpers shared across the crate's Windows modules.

use std::sync::OnceLock;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetPropW, RemovePropW, SetPropW};

/// `text` as a NUL-terminated UTF-16 string.
pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Extract a Win32 `HWND` from any `HasWindowHandle`.
pub(crate) fn hwnd<W: HasWindowHandle + ?Sized>(window: &W) -> Option<HWND> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return None;
    };
    Some(handle.hwnd.get() as HWND)
}

/// A named window property holding one non-zero pointer-sized value per
/// window.
///
/// The UTF-16 property name is encoded once on first use and cached for the
/// process lifetime, so per-message lookups perform zero heap allocations.
///
/// Window hooks find their state through a property instead of
/// `GetWindowSubclass`, which comctl32 v5.82 (the default without a
/// Common-Controls v6 manifest) exports only by ordinal: a by-name import
/// stops every hosted binary from loading.
pub(crate) struct WindowProp {
    name: &'static str,
    utf16: OnceLock<Box<[u16]>>,
}

impl WindowProp {
    pub(crate) const fn new(name: &'static str) -> Self {
        Self {
            name,
            utf16: OnceLock::new(),
        }
    }

    fn ptr(&self) -> *const u16 {
        self.utf16
            .get_or_init(|| wide(self.name).into_boxed_slice())
            .as_ptr()
    }

    /// Store `value`, which must be non-zero to be told apart from an absent
    /// property. Answers whether it was stored.
    pub(crate) fn set(&self, hwnd: HWND, value: usize) -> bool {
        // SAFETY: SetPropW copies the name; hwnd is the caller's live window.
        unsafe { SetPropW(hwnd, self.ptr(), value as _) != 0 }
    }

    /// The stored value; zero when the property is absent.
    pub(crate) fn get(&self, hwnd: HWND) -> usize {
        // SAFETY: GetPropW only reads the property list of hwnd.
        unsafe { GetPropW(hwnd, self.ptr()) as usize }
    }

    pub(crate) fn remove(&self, hwnd: HWND) {
        // SAFETY: RemovePropW only edits the property list of hwnd.
        unsafe { RemovePropW(hwnd, self.ptr()) };
    }
}
