//! The application's icon in the system tray, with its menu.
//!
//! Like the menu bar, the tray is desktop chrome the UI tree cannot draw: a
//! menu-bar extra on macOS, a notification-area icon on Windows. It belongs
//! to the application, not to a window, and it is how an application stays
//! reachable while none of its windows can be clicked. The application
//! describes it with the same [`MenuEntry`](crate::MenuEntry) model as the
//! menu bar and reads the chosen ids from the same queue,
//! [`take_menu_activations`](crate::take_menu_activations).
//!
//! A choice can arrive while the application draws no frames, so
//! [`install_tray`] takes a wake: it runs right after an id is queued, and is
//! where a host signals its event loop.
//!
//! Call these on the thread that runs the event loop. Other platforms report
//! [`TraySupport::Unavailable`] and install nothing.

use std::sync::{Arc, Mutex};

/// What the tray shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tray {
    /// Shown when the pointer rests on the icon.
    pub tooltip: String,
    /// The icon, as an encoded PNG. Square; 32 or 36 pixels suits both
    /// platforms.
    pub icon_png: Vec<u8>,
    /// macOS: treat the icon as a template image, drawn in the menu bar's own
    /// colour from the PNG's alpha. Ignored elsewhere.
    pub template_icon: bool,
    /// The menu the icon opens.
    pub menu: Vec<crate::MenuEntry>,
    /// Windows: the id a primary click reports instead of opening the menu;
    /// `None` opens the menu on either button. macOS always opens the menu.
    pub primary: Option<u32>,
}

/// How the running platform shows a tray.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraySupport {
    /// A menu-bar extra (macOS).
    MenuBarExtra,
    /// A notification-area icon (Windows).
    NotificationArea,
    /// Nothing was installed. Keep the commands reachable in a window.
    Unavailable,
}

/// What [`install_tray`] will do on this platform.
pub const fn tray_support() -> TraySupport {
    #[cfg(target_os = "macos")]
    {
        TraySupport::MenuBarExtra
    }
    #[cfg(target_os = "windows")]
    {
        TraySupport::NotificationArea
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        TraySupport::Unavailable
    }
}

type Wake = Arc<dyn Fn() + Send + Sync>;

static WAKE: Mutex<Option<Wake>> = Mutex::new(None);

/// Runs the installed wake. Called after a tray choice is queued.
#[cfg(any(test, target_os = "macos", target_os = "windows"))]
pub(crate) fn notify() {
    let wake = WAKE.lock().ok().and_then(|wake| wake.clone());
    if let Some(wake) = wake {
        wake();
    }
}

/// Shows `tray`, or brings the tray already shown up to date with it.
///
/// Installing again replaces the menu, tooltip and icon in place: the icon
/// keeps its position. `wake` runs on the event-loop thread right after a
/// chosen id is queued.
///
/// Answers what was installed; a failure to create the icon answers
/// [`TraySupport::Unavailable`].
pub fn install_tray(tray: &Tray, wake: impl Fn() + Send + Sync + 'static) -> TraySupport {
    if let Ok(mut slot) = WAKE.lock() {
        *slot = Some(Arc::new(wake));
    }
    #[cfg(target_os = "macos")]
    {
        if macos::install(tray) {
            return TraySupport::MenuBarExtra;
        }
    }
    #[cfg(target_os = "windows")]
    {
        if windows::install(tray) {
            return TraySupport::NotificationArea;
        }
    }
    let _ = tray;
    if let Ok(mut slot) = WAKE.lock() {
        *slot = None;
    }
    TraySupport::Unavailable
}

/// The tray the platform actually holds: its tooltip and the titles of its
/// menu's entries (`-` for a separator).
///
/// A verification entry, like `installed_menu_bar`. `None` when no tray is
/// shown or the platform cannot be asked.
pub fn installed_tray() -> Option<(String, Vec<String>)> {
    #[cfg(target_os = "macos")]
    {
        macos::installed()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Takes the tray away. Does nothing when none is shown.
pub fn remove_tray() {
    #[cfg(target_os = "macos")]
    macos::remove();
    #[cfg(target_os = "windows")]
    windows::remove();
    if let Ok(mut slot) = WAKE.lock() {
        *slot = None;
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::cell::RefCell;

    use objc2::rc::Retained;
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSImage, NSStatusBar, NSStatusItem, NSVariableStatusItemLength};
    use objc2_foundation::{NSData, NSSize, NSString};

    use super::Tray;

    /// The menu bar's icon height in points.
    const ICON_POINTS: f64 = 18.0;

    thread_local! {
        static ITEM: RefCell<Option<Retained<NSStatusItem>>> = const { RefCell::new(None) };
    }

    pub(super) fn install(tray: &Tray) -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        let item = ITEM.with(|slot| {
            slot.borrow_mut()
                .get_or_insert_with(|| {
                    NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength)
                })
                .clone()
        });
        if let Some(button) = item.button(mtm) {
            let data = NSData::with_bytes(&tray.icon_png);
            if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
                image.setSize(NSSize::new(ICON_POINTS, ICON_POINTS));
                image.setTemplate(tray.template_icon);
                button.setImage(Some(&image));
            }
            button.setToolTip(Some(&NSString::from_str(&tray.tooltip)));
        }
        let menu = crate::platform::build_menu(
            mtm,
            &crate::Menu {
                title: String::new(),
                entries: tray.menu.clone(),
            },
        );
        item.setMenu(Some(&menu));
        true
    }

    pub(super) fn installed() -> Option<(String, Vec<String>)> {
        let mtm = MainThreadMarker::new()?;
        let item = ITEM.with(|slot| slot.borrow().clone())?;
        let tooltip = item
            .button(mtm)
            .and_then(|button| button.toolTip())
            .map(|tip| tip.to_string())
            .unwrap_or_default();
        let menu = item.menu(mtm)?;
        let mut entries = Vec::new();
        for index in 0..menu.numberOfItems() {
            if let Some(entry) = menu.itemAtIndex(index) {
                entries.push(if entry.isSeparatorItem() {
                    "-".to_owned()
                } else {
                    entry.title().to_string()
                });
            }
        }
        Some((tooltip, entries))
    }

    pub(super) fn remove() {
        if let Some(item) = ITEM.with(|slot| slot.borrow_mut().take()) {
            NSStatusBar::systemStatusBar().removeStatusItem(&item);
        }
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use std::cell::RefCell;

    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
        Shell_NotifyIconW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
        DestroyWindow, GetCursorPos, HICON, PostMessageW, RegisterClassW, RegisterWindowMessageW,
        SetForegroundWindow, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP,
        WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
    };

    use super::Tray;
    use crate::win32::wide;

    /// What the shell posts to the tray's window; `lparam` is the mouse
    /// message.
    const TRAY_MESSAGE: u32 = WM_APP + 0x6e;
    const TRAY_ID: u32 = 1;

    struct State {
        hwnd: HWND,
        icon: HICON,
        tooltip: String,
        menu: Vec<crate::MenuEntry>,
        primary: Option<u32>,
        /// Explorer broadcasts this when the taskbar is rebuilt and every
        /// icon has to be added again.
        taskbar_created: u32,
    }

    thread_local! {
        static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    }

    pub(super) fn install(tray: &Tray) -> bool {
        let Some(icon) = icon_from_png(&tray.icon_png) else {
            return false;
        };
        STATE.with(|slot| {
            let mut slot = slot.borrow_mut();
            if let Some(state) = slot.as_mut() {
                let old = std::mem::replace(&mut state.icon, icon);
                state.tooltip.clone_from(&tray.tooltip);
                state.menu.clone_from(&tray.menu);
                state.primary = tray.primary;
                let shown = notify_icon(state, NIM_MODIFY);
                // SAFETY: `old` was created by `icon_from_png` and the shell
                // now holds the new one.
                unsafe { DestroyIcon(old) };
                return shown;
            }
            let Some(hwnd) = create_window() else {
                // SAFETY: created above, never handed to the shell.
                unsafe { DestroyIcon(icon) };
                return false;
            };
            let name = wide("TaskbarCreated");
            let state = State {
                hwnd,
                icon,
                tooltip: tray.tooltip.clone(),
                menu: tray.menu.clone(),
                primary: tray.primary,
                // SAFETY: a NUL-terminated name.
                taskbar_created: unsafe { RegisterWindowMessageW(name.as_ptr()) },
            };
            if !notify_icon(&state, NIM_ADD) {
                // SAFETY: both were created above on this thread.
                unsafe {
                    DestroyWindow(hwnd);
                    DestroyIcon(icon);
                }
                return false;
            }
            *slot = Some(state);
            true
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
        let tip = wide(&state.tooltip);
        // The last slot stays NUL.
        let room = data.szTip.len() - 1;
        for (slot, unit) in data.szTip.iter_mut().zip(tip.iter().take(room)) {
            *slot = *unit;
        }
        // SAFETY: `data` is initialised and outlives the call.
        unsafe { Shell_NotifyIconW(message, &data) != 0 }
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

    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if message == TRAY_MESSAGE {
            match lparam as u32 {
                WM_LBUTTONUP => clicked(hwnd, true),
                WM_RBUTTONUP => clicked(hwnd, false),
                _ => {}
            }
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

    fn clicked(hwnd: HWND, primary_button: bool) {
        // The menu's modal loop dispatches other messages, which may install
        // the tray again: nothing is borrowed while it runs.
        let Some((menu, primary)) = STATE.with(|slot| {
            slot.try_borrow().ok().and_then(|slot| {
                slot.as_ref()
                    .map(|state| (state.menu.clone(), state.primary))
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

    /// A 32-bit icon from the PNG's pixels.
    fn icon_from_png(bytes: &[u8]) -> Option<HICON> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        // A tray icon is small; this bounds what a wrong file can allocate.
        decoder.set_limits(png::Limits { bytes: 4 << 20 });
        let mut reader = decoder.read_info().ok()?;
        let mut raw = vec![0; reader.output_buffer_size()?];
        let frame = reader.next_frame(&mut raw).ok()?;
        let (width, height) = (frame.width as usize, frame.height as usize);
        let channels = match frame.color_type {
            png::ColorType::Rgba => 4,
            png::ColorType::Rgb => 3,
            png::ColorType::GrayscaleAlpha => 2,
            png::ColorType::Grayscale => 1,
            png::ColorType::Indexed => return None,
        };
        if width == 0 || height == 0 || width > 256 || height > 256 {
            return None;
        }
        let mut bgra = Vec::with_capacity(width * height * 4);
        for pixel in raw[..width * height * channels].chunks_exact(channels) {
            let (r, g, b, a) = match *pixel {
                [r, g, b, a] => (r, g, b, a),
                [r, g, b] => (r, g, b, 255),
                [l, a] => (l, l, l, a),
                [l] => (l, l, l, 255),
                _ => return None,
            };
            bgra.extend_from_slice(&[b, g, r, a]);
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
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn a_tray_choice_runs_the_installed_wake_and_removal_drops_it() {
        let _serial = crate::test_serial_lock();
        static WOKEN: AtomicUsize = AtomicUsize::new(0);
        if let Ok(mut slot) = WAKE.lock() {
            *slot = Some(Arc::new(|| {
                WOKEN.fetch_add(1, Ordering::SeqCst);
            }));
        }
        notify();
        assert_eq!(WOKEN.load(Ordering::SeqCst), 1);
        remove_tray();
        notify();
        assert_eq!(WOKEN.load(Ordering::SeqCst), 1);
    }
}
