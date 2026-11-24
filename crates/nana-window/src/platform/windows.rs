use raw_window_handle::HasWindowHandle;
use window_vibrancy::{apply_acrylic, apply_mica, clear_acrylic, clear_mica};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
use windows_sys::Win32::UI::Controls::MARGINS;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, WS_EX_NOREDIRECTIONBITMAP,
};

use crate::win32::wide;
use crate::win32_hooks::{TaskbarHookState, Win32WindowHooks};
use crate::{Appearance, FallbackColor, MaterialEffect, MaterialFallback, MaterialOutcome};

pub(crate) fn apply<W: HasWindowHandle + ?Sized>(
    window: &W,
    requested: MaterialEffect,
    appearance: Appearance,
    fallback: FallbackColor,
) -> MaterialOutcome {
    clear(window);
    if should_reset_extended_frame(requested) {
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

/// Which requests leave an opaque client, so the extended frame an earlier
/// material put on the window has to be undone first.
///
/// `Vibrancy` belongs here because Windows has no such effect: the request ends
/// as a solid outcome, and a window that reports solid must not keep the frame
/// a previous `Transparent` or `Mica` extended across it. Mica and Acrylic undo
/// their own state on the branch where they fail; `Vibrancy` never had a branch
/// that could.
const fn should_reset_extended_frame(requested: MaterialEffect) -> bool {
    matches!(requested, MaterialEffect::Solid | MaterialEffect::Vibrancy)
}

pub(crate) fn set_application_icon_png(_png: &[u8]) {}

/// winit tracks Win32 visibility itself; showing behind it would desync that
/// state, so Windows keeps winit's path.
pub(crate) fn show_without_activation<W: HasWindowHandle + ?Sized>(_window: &W) -> bool {
    false
}

/// Gives a `WS_EX_LAYERED` window that has no layering attributes an opaque
/// constant alpha, and reports whether the window is layered at all.
///
/// winit turns hit-testing off by adding `WS_EX_LAYERED | WS_EX_TRANSPARENT`
/// and never calls `SetLayeredWindowAttributes`. A layered window without
/// attributes is one DWM has nothing to compose for: a window presenting
/// through a DirectComposition visual keeps presenting, but none of it reaches
/// the screen. `LWA_ALPHA` at 255 changes nothing about what the window shows,
/// only that it is shown. Attributes someone else set (a colour key, a real
/// alpha) are left alone.
pub(crate) fn settle_layered_window<W: HasWindowHandle + ?Sized>(window: &W) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetLayeredWindowAttributes, LWA_ALPHA, LWA_COLORKEY, SetLayeredWindowAttributes,
        WS_EX_LAYERED,
    };

    let Some(hwnd) = hwnd(window) else {
        return false;
    };
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    if ex_style & WS_EX_LAYERED as isize == 0 {
        return false;
    }
    let (mut key, mut alpha, mut flags) = (0u32, 0u8, 0u32);
    // SAFETY: `hwnd` is a live layered window; the out pointers are locals.
    let read = unsafe { GetLayeredWindowAttributes(hwnd, &mut key, &mut alpha, &mut flags) };
    if read == 0 || flags & (LWA_ALPHA | LWA_COLORKEY) == 0 {
        // SAFETY: as above.
        unsafe {
            SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
        }
    }
    true
}

/// DWM cloaking: the window stays visible to Win32 (it gets `WM_PAINT`, its
/// swap chain presents and `IsWindowVisible` is true) but is not composed to
/// the screen and is not hit-tested.
pub(crate) fn set_presentation_hold<W: HasWindowHandle + ?Sized>(window: &W, hold: bool) -> bool {
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAK, DwmSetWindowAttribute};

    let Some(hwnd) = hwnd(window) else {
        return false;
    };
    let cloak: i32 = hold.into();
    // SAFETY: `hwnd` is a live window; the attribute takes a BOOL by pointer.
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_CLOAK as u32,
            std::ptr::from_ref(&cloak).cast(),
            std::mem::size_of_val(&cloak) as u32,
        ) >= 0
    }
}

fn apply_solid<W: HasWindowHandle + ?Sized>(window: &W) {
    let Some(hwnd) = hwnd(window) else {
        return;
    };
    extend_frame(hwnd, 0);
}

fn apply_transparent<W: HasWindowHandle + ?Sized>(window: &W) {
    let Some(hwnd) = hwnd(window) else {
        return;
    };
    extend_frame(hwnd, transparent_frame_margin(has_redirection_bitmap(hwnd)));
}

/// Whether DWM still has a redirection bitmap for this HWND.
///
/// `WS_EX_NOREDIRECTIONBITMAP` is a creation flag: winit derives it from
/// `NO_BACK_BUFFER` and `apply_diff` rewrites the whole ex-style, so the bit
/// is only durable when it was set at create. DirectComposition windows are
/// created that way; a plain HWND swapchain keeps the bitmap.
fn has_redirection_bitmap(hwnd: HWND) -> bool {
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    ex_style & WS_EX_NOREDIRECTIONBITMAP as isize == 0
}

/// Glass margin for a transparent client.
///
/// `-1` extends the DWM frame across the client so a redirected swapchain's
/// alpha is visible — that is how a DX12 HWND surface, which only ever
/// advertises `Opaque`, becomes transparent. `0` leaves the frame off: a
/// window without a redirection bitmap presents through a DirectComposition
/// visual that already negotiated premultiplied alpha, and keeping a previous
/// Mica/Acrylic extension would make DWM keep blending a surface that is not
/// there.
const fn transparent_frame_margin(has_redirection_bitmap: bool) -> i32 {
    if has_redirection_bitmap { -1 } else { 0 }
}

/// Extends the frame across the whole client, so DWM reads the alpha of a
/// client that presents through the redirection bitmap. Mica and Acrylic *are*
/// that frame; Transparent uses [`transparent_frame_margin`] instead.
fn prepare_composed_client<W: HasWindowHandle + ?Sized>(window: &W) {
    let Some(hwnd) = hwnd(window) else {
        return;
    };
    extend_frame(hwnd, -1);
}

fn hwnd<W: HasWindowHandle + ?Sized>(window: &W) -> Option<HWND> {
    crate::win32::hwnd(window)
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

#[cfg(test)]
mod tests {
    use super::{should_reset_extended_frame, transparent_frame_margin};
    use crate::MaterialEffect;

    /// A window whose request ends opaque must not be left composed by the
    /// material it is replacing. Windows has no Vibrancy, so that request is
    /// one of them however it reads.
    #[test]
    fn a_request_that_ends_opaque_undoes_the_composed_client() {
        assert!(should_reset_extended_frame(MaterialEffect::Solid));
        assert!(should_reset_extended_frame(MaterialEffect::Vibrancy));
        assert!(!should_reset_extended_frame(MaterialEffect::Transparent));
        assert!(!should_reset_extended_frame(MaterialEffect::Mica));
        assert!(!should_reset_extended_frame(MaterialEffect::Acrylic));
    }

    /// A redirected client borrows DWM alpha; a DirectComposition visual
    /// already has it, and must not keep a leftover extended frame.
    #[test]
    fn transparent_glass_follows_the_redirection_bitmap() {
        assert_eq!(transparent_frame_margin(true), -1);
        assert_eq!(transparent_frame_margin(false), 0);
    }
}

/// Appends `entries` to `popup`, building submenus as it goes.
///
/// # Safety
/// `popup` is a live menu handle owned by the caller.
pub(crate) unsafe fn append_menu_entries(
    popup: windows_sys::Win32::UI::WindowsAndMessaging::HMENU,
    entries: &[crate::MenuEntry],
) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreateMenu, MF_CHECKED, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING,
    };

    for entry in entries {
        match entry {
            crate::MenuEntry::Separator => unsafe {
                AppendMenuW(popup, MF_SEPARATOR, 0, std::ptr::null());
            },
            crate::MenuEntry::Submenu(child) => unsafe {
                let child_popup = CreateMenu();
                append_menu_entries(child_popup, &child.entries);
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
}

/// Builds an `HMENU` from the model and attaches it to the window.
///
/// Windows has no application-level menu bar: the strip belongs to the window,
/// so unlike macOS this needs the handle. Selections arrive as `WM_COMMAND`,
/// which winit forwards to the application as a menu event; the id is the
/// item's command id, matching [`crate::MenuEntry::Item::id`].
pub(crate) fn install_menu_bar<W: HasWindowHandle + ?Sized>(window: &W, bar: &crate::MenuBar) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreateMenu, DrawMenuBar, MF_POPUP, SetMenu,
    };

    let Some(handle) = hwnd(window) else {
        return;
    };

    unsafe {
        let root = CreateMenu();
        for menu in &bar.menus {
            let popup = CreateMenu();
            append_menu_entries(popup, &menu.entries);
            let title = wide(&menu.title);
            AppendMenuW(root, MF_POPUP, popup as usize, title.as_ptr());
        }
        SetMenu(handle, root);
        DrawMenuBar(handle);
        // Without this the menu draws but choosing an item goes nowhere:
        // `WM_COMMAND` reaches winit's window procedure, which has no reason to
        // forward it.
        if let Ok(hooks) = Win32WindowHooks::install_or_get(handle) {
            hooks.menu_enabled.set(true);
        }
    }
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

/// Keeps a window off the taskbar the way the shell itself decides: an owned
/// window without `WS_EX_APPWINDOW` has no button, on every show and after an
/// Explorer restart alike. An unowned window is given a [`taskbar_owner`] of
/// its own.
///
/// Not `ITaskbarList::DeleteTab`: Windows Graphics Capture refuses a window
/// whose button was deleted (`CreateForWindow` returns `E_INVALIDARG`), so
/// window capture in OBS and similar tools would show nothing.
pub(crate) fn set_skip_taskbar<W: HasWindowHandle + ?Sized>(
    window: &W,
    skip: bool,
) -> Result<(), crate::SkipTaskbarError> {
    let Some(hwnd) = hwnd(window) else {
        return Err(crate::SkipTaskbarError::Failed(
            "window has no Win32 handle".into(),
        ));
    };
    if skip {
        hide_taskbar_button(hwnd)
    } else {
        restore_taskbar_button(hwnd)
    }
}

fn hide_taskbar_button(hwnd: HWND) -> Result<(), crate::SkipTaskbarError> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GW_OWNER, GWLP_HWNDPARENT, GetWindow, IsWindowVisible, SW_HIDE, SW_SHOWNA,
        SetWindowLongPtrW, ShowWindow, WS_EX_APPWINDOW,
    };

    if Win32WindowHooks::get(hwnd).is_some_and(|hooks| hooks.taskbar.get().is_some()) {
        return Ok(());
    }
    let failed = |reason: &str| Err(crate::SkipTaskbarError::Failed(reason.into()));
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    let had_button = shell_gives_taskbar_button(hwnd);
    let app_window = ex_style & WS_EX_APPWINDOW as isize != 0;
    // A window that already has an owner (an auxiliary window or a modal
    // dialog) keeps it.
    let owner = if unsafe { GetWindow(hwnd, GW_OWNER) }.is_null() {
        let Some(owner) = taskbar_owner() else {
            return failed("CreateWindowExW failed for the taskbar owner");
        };
        owner
    } else {
        std::ptr::null_mut()
    };
    // Installed first, so a style write from winit cannot slip the bit back
    // in between.
    let hooks = match Win32WindowHooks::install_or_get(hwnd) {
        Ok(hooks) => hooks,
        Err(error) => {
            destroy_taskbar_owner(owner);
            return failed(&error);
        }
    };
    hooks.taskbar.set(Some(TaskbarHookState {
        had_button,
        app_window,
        owner,
    }));
    // The shell reads a window's ownership when it is shown. A shown window
    // given an owner loses its button, but capture keeps refusing it until it
    // is shown again; a hidden one is simply shown without a button later.
    let reshow = had_button && unsafe { IsWindowVisible(hwnd) } != 0;
    // SAFETY: `hwnd` is the caller's live window and `owner` this hook's.
    let owned = unsafe {
        if reshow {
            ShowWindow(hwnd, SW_HIDE);
        }
        if !owner.is_null() {
            SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, owner as isize);
        }
        let owned = owner.is_null() || GetWindow(hwnd, GW_OWNER) == owner;
        if owned && app_window {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style & !(WS_EX_APPWINDOW as isize));
        }
        // Back where it was, without taking activation; winit's own record of
        // the window as shown stays true.
        if reshow {
            ShowWindow(hwnd, SW_SHOWNA);
        }
        owned
    };
    if !owned {
        hooks.taskbar.set(None);
        destroy_taskbar_owner(owner);
        return failed("the taskbar owner was not installed");
    }
    Ok(())
}

fn restore_taskbar_button(hwnd: HWND) -> Result<(), crate::SkipTaskbarError> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GW_OWNER, GWLP_HWNDPARENT, GetWindow, IsWindowVisible, SetWindowLongPtrW, WS_EX_APPWINDOW,
    };

    let Some(hooks) = Win32WindowHooks::get(hwnd) else {
        return Ok(());
    };
    // The guard goes first, or it would strip the bit restored below.
    let Some(state) = hooks.taskbar.take() else {
        return Ok(());
    };
    let owner = state.owner;
    // SAFETY: `hwnd` is the caller's live window and `owner` this hook's.
    unsafe {
        if !owner.is_null() && GetWindow(hwnd, GW_OWNER) == owner {
            SetWindowLongPtrW(hwnd, GWLP_HWNDPARENT, 0);
        }
        // Destroying an owner destroys the windows it owns.
        if GetWindow(hwnd, GW_OWNER) != owner {
            destroy_taskbar_owner(owner);
        }
        if state.app_window {
            let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style | WS_EX_APPWINDOW as isize);
        }
    }
    // Windows the shell never gives a button (such as owned dialogs) stay
    // without one. A hidden window gets its button back on its next show; a
    // shown one is added now, which capture accepts (unlike a deleted one).
    if state.had_button
        && unsafe { IsWindowVisible(hwnd) } != 0
        && let Err(error) = add_taskbar_tab(hwnd)
    {
        // The entry is still hidden: keep the hook, so the state reported
        // stays the state on screen.
        let _ = hide_taskbar_button(hwnd);
        return Err(error);
    }
    Ok(())
}

/// A hidden window to own one window kept off the taskbar, on the calling
/// thread (the window's), so ownership does not join two input queues.
///
/// One per window: windows sharing an owner move together in the z-order, so
/// raising one would raise the others. Never shown, minimized or activated,
/// and sent to the bottom of the z-order, where an owned window is always
/// above it. An ordinary popup rather than a `WS_EX_TOOLWINDOW`: capture also
/// refuses a window whose owner is a tool window.
fn taskbar_owner() -> Option<HWND> {
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, HWND_BOTTOM, RegisterClassW, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOSIZE, SetWindowPos, WNDCLASSW, WS_POPUP,
    };

    let class = wide("NanaTaskbarOwner");
    // SAFETY: the class name and procedure live for the process; a second
    // registration fails harmlessly and the first one is used.
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let definition = WNDCLASSW {
            lpfnWndProc: Some(DefWindowProcW),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            // SAFETY: all-zero is the documented unset state of WNDCLASSW.
            ..std::mem::zeroed()
        };
        RegisterClassW(&definition);
        let owner = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if owner.is_null() {
            return None;
        }
        SetWindowPos(
            owner,
            HWND_BOTTOM,
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
        );
        Some(owner)
    }
}

/// Destroys a [`taskbar_owner`] that no longer owns its window; null is none.
fn destroy_taskbar_owner(owner: HWND) {
    use windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow;

    if !owner.is_null() {
        // SAFETY: a hidden window this module created on this thread.
        unsafe { DestroyWindow(owner) };
    }
}

/// Whether the shell gives this window a taskbar button on its own: an
/// `WS_EX_APPWINDOW` window always has one, otherwise only an unowned window
/// without `WS_EX_TOOLWINDOW`. winit marks owned windows as popups without
/// `WS_EX_APPWINDOW`, so modal dialogs have none.
fn shell_gives_taskbar_button(hwnd: HWND) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GW_OWNER, GetWindow, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    };

    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    if ex_style & WS_EX_APPWINDOW != 0 {
        return true;
    }
    let owned = !unsafe { GetWindow(hwnd, GW_OWNER) }.is_null();
    !owned && ex_style & WS_EX_TOOLWINDOW == 0
}

/// Balances a successful `CoInitializeEx`; a thread already in another
/// apartment can still create the object and is left as it is.
struct ComApartment(bool);

impl ComApartment {
    fn enter() -> Result<Self, crate::SkipTaskbarError> {
        use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
        use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};

        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if hr.is_err() && hr != RPC_E_CHANGED_MODE {
            return Err(crate::SkipTaskbarError::Failed(format!(
                "CoInitializeEx failed: {hr:?}"
            )));
        }
        Ok(Self(hr.is_ok()))
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}

/// Gives a shown window its taskbar button now, instead of on its next show.
fn add_taskbar_tab(hwnd: HWND) -> Result<(), crate::SkipTaskbarError> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::UI::Shell::{ITaskbarList, TaskbarList};

    let _apartment = ComApartment::enter()?;
    let failed = |step: &str, error: windows::core::Error| {
        crate::SkipTaskbarError::Failed(format!("{step}: {error}"))
    };
    let hwnd = windows::Win32::Foundation::HWND(hwnd);
    unsafe {
        let list: ITaskbarList = CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)
            .map_err(|error| failed("CoCreateInstance(TaskbarList)", error))?;
        list.HrInit()
            .map_err(|error| failed("ITaskbarList::HrInit", error))?;
        list.AddTab(hwnd)
            .map_err(|error| failed("ITaskbarList::AddTab", error))
    }
}

/// On real HWNDs: ownership and taskbar eligibility are Win32 window-manager
/// state with no model worth asserting against. Restoring a shown window asks
/// Explorer for a button, so those cases stay with the desktop overlay probe.
#[cfg(test)]
mod taskbar_tests {
    use super::*;
    use raw_window_handle::{HandleError, RawWindowHandle, Win32WindowHandle, WindowHandle};
    use std::num::NonZeroIsize;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, GW_OWNER, GWLP_HWNDPARENT, GetWindow,
        IsWindow, IsWindowVisible, MSG, PM_REMOVE, PeekMessageW, SW_SHOWNA, SetWindowLongPtrW,
        ShowWindow, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW, WS_OVERLAPPEDWINDOW,
    };

    struct Window(HWND);

    impl Window {
        /// A hidden top-level window off every screen, with the taskbar bit
        /// winit gives every unowned window.
        fn new() -> Self {
            // SAFETY: STATIC is a system window class; all pointers are valid
            // for this call.
            let hwnd = unsafe {
                CreateWindowExW(
                    WS_EX_APPWINDOW,
                    wide("STATIC").as_ptr(),
                    wide("taskbar fixture").as_ptr(),
                    WS_OVERLAPPEDWINDOW,
                    -32000,
                    -32000,
                    200,
                    160,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            };
            assert!(!hwnd.is_null(), "fixture window");
            Self(hwnd)
        }

        fn owner(&self) -> HWND {
            // SAFETY: a live fixture window.
            unsafe { GetWindow(self.0, GW_OWNER) }
        }

        fn ex_style(&self) -> u32 {
            ex_style(self.0)
        }
    }

    impl HasWindowHandle for Window {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            let raw = Win32WindowHandle::new(NonZeroIsize::new(self.0 as isize).unwrap());
            // SAFETY: this fixture owns the HWND until Drop on the same thread.
            Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(raw)) })
        }
    }

    impl Drop for Window {
        fn drop(&mut self) {
            // SAFETY: the fixture's own window, on its creating thread.
            unsafe { DestroyWindow(self.0) };
        }
    }

    fn ex_style(hwnd: HWND) -> u32 {
        // SAFETY: `hwnd` is a live window.
        unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 }
    }

    fn alive(hwnd: HWND) -> bool {
        // SAFETY: IsWindow accepts any handle value.
        unsafe { IsWindow(hwnd) != 0 }
    }

    /// Delivers what was posted to this thread's windows.
    fn pump() {
        let mut message = MSG::default();
        // SAFETY: the message stays live through dispatch.
        unsafe {
            while PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                DispatchMessageW(&message);
            }
        }
    }

    #[test]
    fn a_window_off_the_taskbar_is_owned_by_a_hidden_window_of_its_own() {
        let _serial = crate::test_serial_lock();
        let first = Window::new();
        let second = Window::new();

        set_skip_taskbar(&first, true).unwrap();
        set_skip_taskbar(&second, true).unwrap();

        let owner = first.owner();
        assert!(!owner.is_null());
        assert_ne!(
            second.owner(),
            owner,
            "windows sharing an owner move together"
        );
        // SAFETY: a live window.
        assert_eq!(unsafe { IsWindowVisible(owner) }, 0);
        // Capture refuses a window owned by a tool window.
        assert_eq!(ex_style(owner) & WS_EX_TOOLWINDOW, 0);
        assert_eq!(first.ex_style() & WS_EX_APPWINDOW, 0);

        // winit rewrites the extended style from its own flags.
        // SAFETY: a live window.
        unsafe {
            SetWindowLongPtrW(
                first.0,
                GWL_EXSTYLE,
                (first.ex_style() | WS_EX_APPWINDOW) as isize,
            );
        }
        assert_eq!(first.ex_style() & WS_EX_APPWINDOW, 0);

        // Asking again changes nothing.
        set_skip_taskbar(&first, true).unwrap();
        assert_eq!(first.owner(), owner);
    }

    #[test]
    fn showing_the_entry_again_gives_back_what_hiding_it_changed() {
        let _serial = crate::test_serial_lock();
        let window = Window::new();
        set_skip_taskbar(&window, true).unwrap();
        let owner = window.owner();

        set_skip_taskbar(&window, false).unwrap();

        assert!(window.owner().is_null());
        assert!(!alive(owner));
        assert_ne!(window.ex_style() & WS_EX_APPWINDOW, 0);
        // The style guard went with the hook.
        // SAFETY: a live window.
        unsafe {
            SetWindowLongPtrW(
                window.0,
                GWL_EXSTYLE,
                (window.ex_style() & !WS_EX_APPWINDOW) as isize,
            );
            SetWindowLongPtrW(
                window.0,
                GWL_EXSTYLE,
                (window.ex_style() | WS_EX_APPWINDOW) as isize,
            );
        }
        assert_ne!(window.ex_style() & WS_EX_APPWINDOW, 0);
        // A window that was never hidden is left alone.
        set_skip_taskbar(&window, false).unwrap();
        assert!(window.owner().is_null());
    }

    #[test]
    fn a_window_that_already_has_an_owner_keeps_it() {
        let _serial = crate::test_serial_lock();
        let parent = Window::new();
        let window = Window::new();
        // SAFETY: both fixture windows are live.
        unsafe { SetWindowLongPtrW(window.0, GWLP_HWNDPARENT, parent.0 as isize) };

        set_skip_taskbar(&window, true).unwrap();
        assert_eq!(window.owner(), parent.0);
        set_skip_taskbar(&window, false).unwrap();

        assert_eq!(window.owner(), parent.0);
        assert!(alive(parent.0));
    }

    #[test]
    fn a_shown_window_stays_shown() {
        let _serial = crate::test_serial_lock();
        let window = Window::new();
        // SAFETY: a live fixture window, shown off every screen.
        unsafe { ShowWindow(window.0, SW_SHOWNA) };

        set_skip_taskbar(&window, true).unwrap();

        assert!(!window.owner().is_null());
        // SAFETY: a live window.
        assert_ne!(unsafe { IsWindowVisible(window.0) }, 0);
    }

    #[test]
    fn the_owner_is_closed_after_its_window_is_destroyed() {
        let _serial = crate::test_serial_lock();
        let window = Window::new();
        set_skip_taskbar(&window, true).unwrap();
        let owner = window.owner();

        drop(window);
        assert!(alive(owner), "not destroyed while its window is");
        pump();

        assert!(!alive(owner));
    }
}
