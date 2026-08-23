use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use window_vibrancy::{apply_acrylic, apply_mica, clear_acrylic, clear_mica};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
use windows_sys::Win32::UI::Controls::MARGINS;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, WS_EX_NOREDIRECTIONBITMAP,
};

use crate::{Appearance, FallbackColor, MaterialEffect, MaterialFallback, MaterialOutcome};

pub(crate) fn apply<W: HasWindowHandle + ?Sized>(
    window: &W,
    requested: MaterialEffect,
    appearance: Appearance,
    fallback: FallbackColor,
) -> MaterialOutcome {
    clear(window);
    if should_clear_no_redirection_bitmap(requested) {
        apply_solid(window);
    }
    match requested {
        MaterialEffect::Solid => MaterialOutcome::chosen_solid(),
        MaterialEffect::Transparent => {
            apply_transparent(window);
            MaterialOutcome::transparent()
        }
        MaterialEffect::Mica => {
            reset_dwm_margins(window);
            let dark = matches!(appearance, Appearance::Dark);
            if apply_mica(window, Some(dark)).is_ok() {
                MaterialOutcome::native(MaterialEffect::Mica)
            } else {
                MaterialOutcome::solid(MaterialFallback::NativeMaterialUnavailable)
            }
        }
        MaterialEffect::Acrylic => {
            reset_dwm_margins(window);
            if apply_acrylic(window, Some(fallback.tuple())).is_ok() {
                MaterialOutcome::native(MaterialEffect::Acrylic)
            } else {
                MaterialOutcome::solid(MaterialFallback::NativeMaterialUnavailable)
            }
        }
        MaterialEffect::Vibrancy => {
            MaterialOutcome::solid(MaterialFallback::PlatformDoesNotProvideNativeMaterial)
        }
    }
}

pub(crate) fn clear<W: HasWindowHandle + ?Sized>(window: &W) {
    let _ = clear_mica(window);
    let _ = clear_acrylic(window);
}

const fn should_clear_no_redirection_bitmap(requested: MaterialEffect) -> bool {
    matches!(requested, MaterialEffect::Solid)
}

pub(crate) fn set_application_icon_png(_png: &[u8]) {}

fn apply_solid<W: HasWindowHandle + ?Sized>(window: &W) {
    let Some(hwnd) = hwnd(window) else {
        return;
    };
    extend_frame(hwnd, 0);
    set_no_redirection_bitmap(hwnd, false);
}

fn apply_transparent<W: HasWindowHandle + ?Sized>(window: &W) {
    let Some(hwnd) = hwnd(window) else {
        return;
    };
    extend_frame(hwnd, -1);
    set_no_redirection_bitmap(hwnd, true);
}

fn reset_dwm_margins<W: HasWindowHandle + ?Sized>(window: &W) {
    let Some(hwnd) = hwnd(window) else {
        return;
    };
    extend_frame(hwnd, 0);
}

fn hwnd<W: HasWindowHandle + ?Sized>(window: &W) -> Option<HWND> {
    let handle = window.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as HWND),
        _ => None,
    }
}

fn extend_frame(hwnd: HWND, margin: i32) {
    let margins = MARGINS {
        cxLeftWidth: margin,
        cxRightWidth: margin,
        cyTopHeight: margin,
        cyBottomHeight: margin,
    };
    unsafe {
        let _ = DwmExtendFrameIntoClientArea(hwnd, &margins);
    }
}

fn set_no_redirection_bitmap(hwnd: HWND, enabled: bool) {
    unsafe {
        let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let bit = WS_EX_NOREDIRECTIONBITMAP as isize;
        let next = if enabled {
            current | bit
        } else {
            current & !bit
        };
        if next == current {
            return;
        }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, next);
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::should_clear_no_redirection_bitmap;
    use crate::MaterialEffect;

    #[test]
    fn only_solid_clears_no_redirection_bitmap() {
        assert!(should_clear_no_redirection_bitmap(MaterialEffect::Solid));
        assert!(!should_clear_no_redirection_bitmap(
            MaterialEffect::Transparent
        ));
        assert!(!should_clear_no_redirection_bitmap(MaterialEffect::Mica));
        assert!(!should_clear_no_redirection_bitmap(MaterialEffect::Acrylic));
        assert!(!should_clear_no_redirection_bitmap(
            MaterialEffect::Vibrancy
        ));
    }
}
