//! Small Win32 helpers shared across the crate's Windows modules.

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{GetPropW, RemovePropW, SetPropW};

/// `text` as a NUL-terminated UTF-16 string.
pub(crate) fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A named window property holding one non-zero pointer-sized value per
/// window.
///
/// Window hooks find their state through a property instead of
/// `GetWindowSubclass`, which comctl32 v5.82 (the default without a
/// Common-Controls v6 manifest) exports only by ordinal: a by-name import
/// stops every hosted binary from loading.
pub(crate) struct WindowProp(&'static str);

impl WindowProp {
    pub(crate) const fn new(name: &'static str) -> Self {
        Self(name)
    }

    /// Store `value`, which must be non-zero to be told apart from an absent
    /// property. Answers whether it was stored.
    pub(crate) fn set(&self, hwnd: HWND, value: usize) -> bool {
        let name = wide(self.0);
        // SAFETY: SetPropW copies the name; hwnd is the caller's live window.
        unsafe { SetPropW(hwnd, name.as_ptr(), value as _) != 0 }
    }

    /// The stored value; zero when the property is absent.
    pub(crate) fn get(&self, hwnd: HWND) -> usize {
        let name = wide(self.0);
        // SAFETY: GetPropW only reads the property list of hwnd.
        unsafe { GetPropW(hwnd, name.as_ptr()) as usize }
    }

    pub(crate) fn remove(&self, hwnd: HWND) {
        let name = wide(self.0);
        // SAFETY: RemovePropW only edits the property list of hwnd.
        unsafe { RemovePropW(hwnd, name.as_ptr()) };
    }
}
