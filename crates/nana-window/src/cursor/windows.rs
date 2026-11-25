use std::ffi::c_void;
use std::ptr;
use std::sync::Arc;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateCompatibleBitmap, DeleteObject, GetDC, ReleaseDC, SetBitmapBits,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyCursor, HCURSOR, ICONINFO, SetCursor,
};

use super::{CursorBitmap, CustomCursorCapabilities, CustomCursorOutcome};
use crate::win32_hooks::Win32WindowHooks;

pub(crate) struct OwnedHcursor {
    handle: HCURSOR,
}

unsafe impl Send for OwnedHcursor {}
unsafe impl Sync for OwnedHcursor {}

impl Drop for OwnedHcursor {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { DestroyCursor(self.handle) };
        }
    }
}

impl OwnedHcursor {
    pub(crate) fn raw(&self) -> HCURSOR {
        self.handle
    }
}

#[derive(Clone)]
pub(super) struct WindowsCursorHandle {
    owned: Arc<OwnedHcursor>,
}

pub(super) fn capabilities() -> CustomCursorCapabilities {
    CustomCursorCapabilities {
        supported: true,
        multi_dpi: true,
        max_edge: 256,
    }
}

pub(super) fn create_cursor(bitmap: &CursorBitmap) -> Result<WindowsCursorHandle, String> {
    let mut bgra = bitmap.rgba.to_vec();
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let w = i32::from(bitmap.width);
    let h = i32::from(bitmap.height);
    unsafe {
        let hdc_screen = GetDC(ptr::null_mut());
        if hdc_screen.is_null() {
            return Err("GetDC failed while creating custom cursor".into());
        }
        let hbm_color = CreateCompatibleBitmap(hdc_screen, w, h);
        ReleaseDC(ptr::null_mut(), hdc_screen);
        if hbm_color.is_null() {
            return Err("CreateCompatibleBitmap failed for custom cursor".into());
        }
        if SetBitmapBits(hbm_color, bgra.len() as u32, bgra.as_ptr() as *const c_void) == 0 {
            DeleteObject(hbm_color);
            return Err("SetBitmapBits failed for custom cursor".into());
        }
        let mask_stride = ((w + 15) >> 4) << 1;
        let mask_bits = vec![0xffu8; (mask_stride * h) as usize];
        let hbm_mask = CreateBitmap(w, h, 1, 1, mask_bits.as_ptr() as *const c_void);
        if hbm_mask.is_null() {
            DeleteObject(hbm_color);
            return Err("CreateBitmap failed for custom cursor mask".into());
        }
        let icon_info = ICONINFO {
            fIcon: 0,
            xHotspot: u32::from(bitmap.hotspot[0]),
            yHotspot: u32::from(bitmap.hotspot[1]),
            hbmMask: hbm_mask,
            hbmColor: hbm_color,
        };
        let handle = CreateIconIndirect(&icon_info);
        DeleteObject(hbm_color);
        DeleteObject(hbm_mask);
        if handle.is_null() {
            return Err("CreateIconIndirect failed for custom cursor".into());
        }
        Ok(WindowsCursorHandle {
            owned: Arc::new(OwnedHcursor { handle }),
        })
    }
}

fn hwnd_of<W: HasWindowHandle + ?Sized>(window: &W) -> Option<HWND> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return None;
    };
    Some(handle.hwnd.get() as HWND)
}

pub(super) fn apply_custom_cursor<W: HasWindowHandle + ?Sized>(
    window: &W,
    handle: &WindowsCursorHandle,
    scale_milli: u32,
) -> CustomCursorOutcome {
    let Some(hwnd) = hwnd_of(window) else {
        return CustomCursorOutcome::Unsupported("missing Win32 HWND".into());
    };
    let Ok(hooks) = Win32WindowHooks::install_or_get(hwnd) else {
        return CustomCursorOutcome::Fallback("failed to install Win32 cursor hook".into());
    };
    *hooks.custom_cursor.borrow_mut() = Some(Arc::clone(&handle.owned));
    unsafe { SetCursor(handle.owned.raw()) };
    CustomCursorOutcome::Applied { scale_milli }
}

pub(super) fn clear_custom_cursor<W: HasWindowHandle + ?Sized>(window: &W) {
    let Some(hwnd) = hwnd_of(window) else {
        return;
    };
    if let Some(hooks) = Win32WindowHooks::get(hwnd) {
        *hooks.custom_cursor.borrow_mut() = None;
    }
}
