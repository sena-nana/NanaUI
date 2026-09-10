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
            prepare_composed_client(window);
            let dark = matches!(appearance, Appearance::Dark);
            if apply_mica(window, Some(dark)).is_ok() {
                MaterialOutcome::native(MaterialEffect::Mica)
            } else {
                apply_solid(window);
                MaterialOutcome::solid(MaterialFallback::NativeMaterialUnavailable)
            }
        }
        MaterialEffect::Acrylic => {
            prepare_composed_client(window);
            if apply_acrylic(window, Some(fallback.tuple())).is_ok() {
                MaterialOutcome::native(MaterialEffect::Acrylic)
            } else {
                apply_solid(window);
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
    prepare_composed_client(window);
}

fn prepare_composed_client<W: HasWindowHandle + ?Sized>(window: &W) {
    let Some(hwnd) = hwnd(window) else {
        return;
    };
    extend_frame(hwnd, -1);
    set_no_redirection_bitmap(hwnd, true);
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

/// Builds an `HMENU` from the model and attaches it to the window.
///
/// Windows has no application-level menu bar: the strip belongs to the window,
/// so unlike macOS this needs the handle. Selections arrive as `WM_COMMAND`,
/// which winit forwards to the application as a menu event; the id is the
/// item's command id, matching [`crate::MenuEntry::Item::id`].
pub(crate) fn install_menu_bar<W: HasWindowHandle + ?Sized>(window: &W, bar: &crate::MenuBar) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreateMenu, DrawMenuBar, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR,
        MF_STRING, SetMenu,
    };

    let Some(handle) = hwnd(window) else {
        return;
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Appends one level of the model. Returns the built popup.
    unsafe fn build(menu: &crate::Menu) -> windows_sys::Win32::UI::WindowsAndMessaging::HMENU {
        let popup = unsafe { CreateMenu() };
        for entry in &menu.entries {
            match entry {
                crate::MenuEntry::Separator => unsafe {
                    AppendMenuW(popup, MF_SEPARATOR, 0, std::ptr::null());
                },
                crate::MenuEntry::Submenu(child) => unsafe {
                    let child_popup = build(child);
                    let title = wide(&child.title);
                    AppendMenuW(popup, MF_POPUP, child_popup as usize, title.as_ptr());
                },
                crate::MenuEntry::Item {
                    id,
                    label,
                    shortcut,
                    enabled,
                    checked,
                } => unsafe {
                    // Windows draws the accelerator from the label, so the
                    // shortcut is appended after a tab the way the platform
                    // expects. Claiming the key itself is the host's job.
                    let mut text = label.clone();
                    if let Some(shortcut) = shortcut {
                        let mut parts = Vec::new();
                        if shortcut.primary {
                            parts.push("Ctrl");
                        }
                        if shortcut.shift {
                            parts.push("Shift");
                        }
                        if shortcut.alt {
                            parts.push("Alt");
                        }
                        let key = shortcut.key.to_uppercase();
                        parts.push(&key);
                        text.push('\t');
                        text.push_str(&parts.join("+"));
                    }
                    let mut flags = MF_STRING;
                    if !*enabled {
                        flags |= MF_GRAYED;
                    }
                    if *checked {
                        flags |= MF_CHECKED;
                    }
                    let text = wide(&text);
                    AppendMenuW(popup, flags, *id as usize, text.as_ptr());
                },
            }
        }
        popup
    }

    unsafe {
        let root = CreateMenu();
        for menu in &bar.menus {
            let popup = build(menu);
            let title = wide(&menu.title);
            AppendMenuW(root, MF_POPUP, popup as usize, title.as_ptr());
        }
        SetMenu(handle, root);
        DrawMenuBar(handle);
        // Without this the menu draws but choosing an item goes nowhere:
        // `WM_COMMAND` reaches winit's window procedure, which has no reason to
        // forward it.
        attach_menu_subclass(handle);
    }
}

/// Subclass id for the menu hook. Any constant unique within this window.
const MENU_SUBCLASS_ID: usize = 0x6e61_6e61;

/// Installs the `WM_COMMAND` hook once per window.
///
/// `SetWindowSubclass` is idempotent for a given (window, proc, id) triple: a
/// second call replaces the existing entry rather than stacking, so
/// re-installing the menu does not chain hooks.
unsafe fn attach_menu_subclass(hwnd: HWND) {
    use windows_sys::Win32::UI::Shell::SetWindowSubclass;

    unsafe {
        SetWindowSubclass(hwnd, Some(menu_subclass_proc), MENU_SUBCLASS_ID, 0);
    }
}

unsafe extern "system" fn menu_subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: usize,
    lparam: isize,
    _id: usize,
    _data: usize,
) -> isize {
    use windows_sys::Win32::UI::Shell::DefSubclassProc;
    use windows_sys::Win32::UI::WindowsAndMessaging::WM_COMMAND;

    // A menu selection arrives with a null lparam and the command id in the
    // low word of wparam; anything else (accelerators, controls) is not ours.
    if message == WM_COMMAND && lparam == 0 && (wparam >> 16) & 0xffff == 0 {
        let id = (wparam & 0xffff) as u32;
        if id != 0 {
            crate::menu::push_activation(id);
            return 0;
        }
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

/// The worker owns a CBT hook so cancellation before the picker is created
/// also closes it when it activates. Later cancellation closes that thread's
/// dialog directly; the host never blocks on the native modal loop.
#[derive(Clone, Default)]
pub(crate) struct DialogCancellation {
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: std::sync::Arc<std::sync::Mutex<u32>>,
}
thread_local! {
    static DIALOG_CANCEL: std::cell::RefCell<Option<DialogCancellation>> = const { std::cell::RefCell::new(None) };
}
pub(crate) struct DialogHook {
    hook: windows_sys::Win32::UI::WindowsAndMessaging::HHOOK,
    cancellation: DialogCancellation,
}
impl DialogCancellation {
    pub(crate) fn cancelled(&self) -> bool {
        self.cancelled.load(std::sync::atomic::Ordering::SeqCst)
    }
    pub(crate) fn cancel(&self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::SeqCst);
        // Hold registration while enumerating: the worker cannot exit and
        // let Windows reuse its thread ID for an unrelated picker.
        let thread = self
            .thread
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if *thread != 0 {
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::EnumThreadWindows(
                    *thread,
                    Some(close_picker),
                    0,
                );
            }
        }
    }
    pub(crate) fn install(&self) -> Result<DialogHook, nana_ui_core::FileDialogError> {
        use windows_sys::Win32::{
            System::Threading::GetCurrentThreadId,
            UI::WindowsAndMessaging::{SetWindowsHookExW, WH_CBT},
        };
        let thread = unsafe { GetCurrentThreadId() };
        DIALOG_CANCEL.with(|slot| *slot.borrow_mut() = Some(self.clone()));
        let hook =
            unsafe { SetWindowsHookExW(WH_CBT, Some(dialog_hook), std::ptr::null_mut(), thread) };
        if hook.is_null() {
            DIALOG_CANCEL.with(|slot| slot.borrow_mut().take());
            return Err(nana_ui_core::FileDialogError::Platform(
                std::io::Error::last_os_error().to_string(),
            ));
        }
        *self
            .thread
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = thread;
        Ok(DialogHook {
            hook,
            cancellation: self.clone(),
        })
    }
}
impl Drop for DialogHook {
    fn drop(&mut self) {
        *self
            .cancellation
            .thread
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = 0;
        DIALOG_CANCEL.with(|slot| slot.borrow_mut().take());
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::UnhookWindowsHookEx(self.hook);
        }
    }
}
unsafe extern "system" fn close_picker(
    window: windows_sys::Win32::Foundation::HWND,
    _: isize,
) -> i32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClassNameW, PostMessageW, WM_CLOSE};
    let mut class = [0u16; 32];
    let length = unsafe { GetClassNameW(window, class.as_mut_ptr(), class.len() as i32) };
    if length == 6 && class[..6] == [35, 51, 50, 55, 55, 48] {
        unsafe {
            PostMessageW(window, WM_CLOSE, 0, 0);
        }
    }
    1
}
unsafe extern "system" fn dialog_hook(code: i32, wparam: usize, lparam: isize) -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::{CallNextHookEx, HCBT_ACTIVATE};
    if code == HCBT_ACTIVATE as i32
        && DIALOG_CANCEL.with(|slot| {
            slot.borrow()
                .as_ref()
                .is_some_and(DialogCancellation::cancelled)
        })
    {
        unsafe {
            close_picker(wparam as _, 0);
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}
