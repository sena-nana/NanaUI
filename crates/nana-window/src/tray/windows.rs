use std::cell::RefCell;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, KEY_READ, RRF_RT_REG_DWORD, RegCloseKey, RegGetValueW, RegOpenKeyExW,
};
use windows_sys::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, GetCursorPos, GetSystemMetrics, HICON, PostMessageW, RegisterClassW,
    RegisterWindowMessageW, SM_CXSMICON, SetForegroundWindow, TPM_NONOTIFY, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP, WM_DPICHANGED, WM_LBUTTONDBLCLK, WM_LBUTTONUP,
    WM_NULL, WM_RBUTTONUP, WM_SETTINGCHANGE, WNDCLASSW,
};

use super::{
    Tray, TrayAttention, decode_png_rgba, resize_rgba_square, select_tray_icon_bytes,
    summarize_menu_entries,
};
use crate::win32::wide;

/// What the shell posts to the tray's window; `lparam` is the mouse message.
const TRAY_MESSAGE: u32 = WM_APP + 0x6e;
const TRAY_ID: u32 = 1;

struct State {
    hwnd: HWND,
    icon: HICON,
    tray: Tray,
    /// Explorer broadcasts this when the taskbar is rebuilt and every icon
    /// has to be added again.
    taskbar_created: u32,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

pub(super) fn install(tray: &Tray) -> Result<(), String> {
    STATE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(state) = slot.as_mut() {
            let icon = icon_for_tray(tray, Some(state.hwnd))
                .ok_or_else(|| "invalid tray PNG icon bytes".to_string())?;
            let old = std::mem::replace(&mut state.icon, icon);
            state.tray = tray.clone();
            let shown = notify_icon(state, NIM_MODIFY);
            // SAFETY: `old` was created by `icon_for_tray` and the shell now
            // holds the new one.
            unsafe { DestroyIcon(old) };
            return if shown {
                Ok(())
            } else {
                Err("Shell_NotifyIconW(NIM_MODIFY) failed".into())
            };
        }
        let hwnd = create_window().ok_or_else(|| "CreateWindowExW failed for tray".to_string())?;
        let Some(icon) = icon_for_tray(tray, Some(hwnd)) else {
            unsafe { DestroyWindow(hwnd) };
            return Err("invalid tray PNG icon bytes".into());
        };
        let name = wide("TaskbarCreated");
        let state = State {
            hwnd,
            icon,
            tray: tray.clone(),
            // SAFETY: a NUL-terminated name.
            taskbar_created: unsafe { RegisterWindowMessageW(name.as_ptr()) },
        };
        if !notify_icon(&state, NIM_ADD) {
            // SAFETY: both were created above on this thread.
            unsafe {
                DestroyWindow(hwnd);
                DestroyIcon(icon);
            }
            return Err("Shell_NotifyIconW(NIM_ADD) failed".into());
        }
        *slot = Some(state);
        Ok(())
    })
}

pub(super) fn set_attention(attention: TrayAttention) -> bool {
    STATE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(state) = slot.as_mut() else {
            return false;
        };
        state.tray.attention = attention;
        notify_icon(state, NIM_MODIFY)
    })
}

/// The tray while the shell still places its icon: the shell cannot report
/// the tooltip or menu back, so those come from what was installed, gated on
/// `Shell_NotifyIconGetRect` finding the icon.
pub(super) fn installed() -> Option<(String, Vec<String>)> {
    STATE.with(|slot| {
        let slot = slot.try_borrow().ok()?;
        let state = slot.as_ref()?;
        // SAFETY: plain data; zero is its empty value.
        let mut identifier: NOTIFYICONIDENTIFIER = unsafe { std::mem::zeroed() };
        identifier.cbSize = std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32;
        identifier.hWnd = state.hwnd;
        identifier.uID = TRAY_ID;
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        // SAFETY: both pointers are valid for the call.
        let held = unsafe { Shell_NotifyIconGetRect(&identifier, &mut rect) } == 0;
        held.then(|| {
            (
                state.tray.tooltip.clone(),
                summarize_menu_entries(&state.tray.menu),
            )
        })
    })
}

pub(super) fn remove() {
    if let Some(state) = STATE.with(|slot| slot.borrow_mut().take()) {
        notify_icon(&state, NIM_DELETE);
        // SAFETY: created by `install` on this thread.
        unsafe {
            DestroyWindow(state.hwnd);
            DestroyIcon(state.icon);
        }
    }
}

fn notify_icon(state: &State, message: u32) -> bool {
    // SAFETY: NOTIFYICONDATAW is plain data; zero is its empty value.
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = state.hwnd;
    data.uID = TRAY_ID;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = TRAY_MESSAGE;
    data.hIcon = state.icon;
    let tip = wide(&state.tray.tooltip);
    // The last slot stays NUL.
    let room = data.szTip.len() - 1;
    for (slot, unit) in data.szTip.iter_mut().zip(tip.iter().take(room)) {
        *slot = *unit;
    }
    // SAFETY: `data` is initialised and outlives the call.
    unsafe { Shell_NotifyIconW(message, &data) != 0 }
}

fn refresh_icon(hwnd: HWND) {
    STATE.with(|slot| {
        let Ok(mut slot) = slot.try_borrow_mut() else {
            return;
        };
        let Some(state) = slot.as_mut() else {
            return;
        };
        let Some(icon) = icon_for_tray(&state.tray, Some(hwnd)) else {
            return;
        };
        let old = std::mem::replace(&mut state.icon, icon);
        notify_icon(state, NIM_MODIFY);
        // SAFETY: `old` was replaced above.
        unsafe { DestroyIcon(old) };
    });
}

/// Reads Windows `SystemUsesLightTheme` (taskbar light theme) so light/dark
/// monochrome tray variants follow Windows 10/11 taskbar appearance.
fn taskbar_uses_light_theme() -> bool {
    let subkey = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let value = wide("SystemUsesLightTheme");
    unsafe {
        let mut hkey = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_READ, &mut hkey) != 0 {
            return false;
        }
        let mut data: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        let status = RegGetValueW(
            hkey,
            std::ptr::null(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut data as *mut u32).cast(),
            &mut size,
        );
        RegCloseKey(hkey);
        status == 0 && data != 0
    }
}

fn small_icon_edge(hwnd: Option<HWND>) -> u32 {
    let metric = unsafe {
        if let Some(hwnd) = hwnd {
            let dpi = GetDpiForWindow(hwnd);
            if dpi != 0 {
                GetSystemMetricsForDpi(SM_CXSMICON, dpi)
            } else {
                GetSystemMetrics(SM_CXSMICON)
            }
        } else {
            GetSystemMetrics(SM_CXSMICON)
        }
    };
    u32::try_from(metric).unwrap_or(16).max(16)
}

fn icon_for_tray(tray: &Tray, hwnd: Option<HWND>) -> Option<HICON> {
    let light_taskbar = taskbar_uses_light_theme();
    let png_bytes = select_tray_icon_bytes(tray, light_taskbar);
    let (w, h, rgba) = decode_png_rgba(png_bytes.as_ref())?;
    let target = small_icon_edge(hwnd);
    let (w, h, rgba) = if w != h || w > target.saturating_mul(2) {
        resize_rgba_square(w, h, &rgba, target)
    } else {
        (w, h, rgba)
    };
    icon_from_rgba(w as usize, h as usize, &rgba)
}

/// A hidden top-level window: a message-only window would not receive the
/// `TaskbarCreated` broadcast.
fn create_window() -> Option<HWND> {
    let class = wide("NanaTrayWindow");
    // SAFETY: the class name and procedure live for the process; a second
    // registration fails harmlessly and the first one is used.
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let mut definition: WNDCLASSW = std::mem::zeroed();
        definition.lpfnWndProc = Some(window_proc);
        definition.hInstance = instance;
        definition.lpszClassName = class.as_ptr();
        RegisterClassW(&definition);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        (!hwnd.is_null()).then_some(hwnd)
    }
}

fn lparam_matches_immersive_color_set(lparam: LPARAM) -> bool {
    if lparam == 0 {
        return false;
    }
    let ptr = lparam as *const u16;
    let target = wide("ImmersiveColorSet");
    // Compare up to `target.len()` UTF-16 code units (including NUL).
    unsafe {
        for (i, &unit) in target.iter().enumerate() {
            if *ptr.add(i) != unit {
                return false;
            }
        }
        true
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == TRAY_MESSAGE {
        match lparam as u32 {
            WM_LBUTTONUP => clicked(hwnd, true),
            WM_LBUTTONDBLCLK => double_clicked(),
            WM_RBUTTONUP => clicked(hwnd, false),
            _ => {}
        }
        return 0;
    }
    if message == WM_DPICHANGED
        || (message == WM_SETTINGCHANGE && lparam_matches_immersive_color_set(lparam))
    {
        refresh_icon(hwnd);
        return 0;
    }
    let readded = STATE.with(|slot| {
        // A re-entrant install holds the state; it adds the icon itself.
        let Ok(slot) = slot.try_borrow() else {
            return false;
        };
        match slot.as_ref() {
            Some(state) if state.taskbar_created != 0 && message == state.taskbar_created => {
                notify_icon(state, NIM_ADD);
                true
            }
            _ => false,
        }
    });
    if readded {
        return 0;
    }
    // SAFETY: forwarding the procedure's own arguments.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn double_clicked() {
    let double_click = STATE.with(|slot| {
        slot.try_borrow()
            .ok()
            .and_then(|slot| slot.as_ref().and_then(|state| state.tray.double_click))
    });
    if let Some(id) = double_click {
        crate::menu::push_activation(id);
        super::notify();
    }
}

fn clicked(hwnd: HWND, primary_button: bool) {
    // The menu's modal loop dispatches other messages, which may install the
    // tray again: nothing is borrowed while it runs.
    let Some((menu, primary)) = STATE.with(|slot| {
        slot.try_borrow().ok().and_then(|slot| {
            slot.as_ref()
                .map(|state| (state.tray.menu.clone(), state.tray.primary))
        })
    }) else {
        return;
    };
    if let (true, Some(id)) = (primary_button, primary) {
        crate::menu::push_activation(id);
        super::notify();
        return;
    }
    let chosen = unsafe {
        let popup = CreatePopupMenu();
        if popup.is_null() {
            return;
        }
        crate::platform::append_menu_entries(popup, &menu);
        let mut point = POINT { x: 0, y: 0 };
        GetCursorPos(&mut point);
        // Without foreground the menu would not close on a click outside.
        SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenu(
            popup,
            TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            hwnd,
            std::ptr::null(),
        );
        DestroyMenu(popup);
        // The documented nudge that lets the menu dismiss cleanly.
        PostMessageW(hwnd, WM_NULL, 0, 0);
        chosen
    };
    if let Ok(id) = u32::try_from(chosen)
        && id != 0
    {
        crate::menu::push_activation(id);
        super::notify();
    }
}

fn icon_from_rgba(width: usize, height: usize, rgba: &[u8]) -> Option<HICON> {
    let mut bgra = Vec::with_capacity(width * height * 4);
    for px in rgba.chunks_exact(4) {
        bgra.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }
    // One bit per pixel, rows padded to 16 bits; alpha carries the shape.
    let mask = vec![0u8; width.div_ceil(16) * 2 * height];
    // SAFETY: both buffers hold the sizes the arguments describe and are
    // copied by the call.
    let icon = unsafe {
        CreateIcon(
            GetModuleHandleW(std::ptr::null()),
            width as i32,
            height as i32,
            1,
            32,
            mask.as_ptr(),
            bgra.as_ptr(),
        )
    };
    (!icon.is_null()).then_some(icon)
}
