//! Installs a tray and reads back what the platform actually holds.
//!
//! Runs on the main thread, which the tray needs, so it verifies the real
//! platform path. Prints the read-back and exits non-zero when it does not
//! match what was asked for. Pass `--hold <seconds>` to keep the tray up for
//! a look. On Windows it also drives the tray window's live message paths:
//! a double-click activation, Explorer's `TaskbarCreated` re-add after the
//! shell dropped the icon, and the theme and DPI icon refreshes.

use nana_window::{
    MenuEntry, Tray, TraySupport, install_tray, installed_tray, remove_tray, tray_support,
};

/// A 2×2 opaque square: enough for the platform to accept an icon.
const ICON: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x06, 0x00, 0x00, 0x00, 0x72, 0xb6, 0x0d,
    0x24, 0x00, 0x00, 0x00, 0x10, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x60, 0x60, 0x60, 0xf8,
    0x0f, 0xc5, 0x10, 0x06, 0x00, 0x1d, 0xf4, 0x03, 0xfd, 0xc8, 0xe1, 0xf6, 0xa8, 0x00, 0x00, 0x00,
    0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn tray(checked: bool) -> Tray {
    Tray {
        tooltip: "Tray probe".into(),
        icon_png: ICON.to_vec(),
        template_icon: true,
        menu: vec![
            MenuEntry::item(1, "Show"),
            MenuEntry::Separator,
            MenuEntry::item(2, "Click-through").checked(checked),
            MenuEntry::item(3, "Quit"),
        ],
        primary: Some(1),
        double_click: Some(4),
        ..Default::default()
    }
}

fn main() -> std::process::ExitCode {
    println!("support: {:?}", tray_support());
    let outcome = install_tray(&tray(false), || {});
    println!("install: {outcome:?}");
    if outcome == TraySupport::Unavailable {
        println!("no tray on this platform; nothing to verify");
        remove_tray();
        return std::process::ExitCode::SUCCESS;
    }
    // Installing again updates the same item in place.
    install_tray(&tray(true), || {});
    let expected = (
        "Tray probe".to_owned(),
        vec![
            "Show".to_owned(),
            "-".to_owned(),
            "Click-through".to_owned(),
            "Quit".to_owned(),
        ],
    );
    let installed = installed_tray();
    println!("installed: {installed:?}");
    #[cfg(target_os = "windows")]
    if let Err(failure) = windows_live::exercise() {
        remove_tray();
        eprintln!("FAIL: {failure}");
        return std::process::ExitCode::FAILURE;
    }
    let mut arguments = std::env::args().skip(1);
    if arguments.next().as_deref() == Some("--hold")
        && let Some(seconds) = arguments.next().and_then(|value| value.parse().ok())
    {
        std::thread::sleep(std::time::Duration::from_secs(seconds));
    }
    remove_tray();
    if installed.as_ref() != Some(&expected) {
        eprintln!("FAIL: expected {expected:?}");
        return std::process::ExitCode::FAILURE;
    }
    if installed_tray().is_some() {
        eprintln!("FAIL: the tray is still held after removal");
        return std::process::ExitCode::FAILURE;
    }
    println!("OK: the platform holds exactly the tray that was installed, and lets it go");
    std::process::ExitCode::SUCCESS
}

#[cfg(target_os = "windows")]
mod windows_live {
    use std::time::{Duration, Instant};

    use nana_window::{installed_tray, take_menu_activations};
    use windows_sys::Win32::UI::Shell::{NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, FindWindowW, MSG, PM_REMOVE, PeekMessageW, PostMessageW,
        RegisterWindowMessageW, SendMessageW, TranslateMessage, WM_APP, WM_DPICHANGED,
        WM_LBUTTONDBLCLK, WM_SETTINGCHANGE,
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    /// Dispatches this thread's messages for a bounded moment.
    fn pump(duration: Duration) {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            // SAFETY: a zeroed MSG is a valid out-parameter.
            let mut message: MSG = unsafe { std::mem::zeroed() };
            while unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0
            {
                unsafe {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn exercise() -> Result<(), String> {
        let class = wide("NanaTrayWindow");
        // SAFETY: a NUL-terminated class name.
        let hwnd = unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) };
        if hwnd.is_null() {
            return Err("the tray window was not found".into());
        }
        take_menu_activations();

        // The shell reports a double-click as the mouse message in `lparam`.
        unsafe { PostMessageW(hwnd, WM_APP + 0x6e, 1, WM_LBUTTONDBLCLK as isize) };
        pump(Duration::from_millis(200));
        let activations = take_menu_activations();
        println!("double-click activations: {activations:?}");
        if !activations.contains(&4) {
            return Err("a double-click did not activate the double_click entry".into());
        }

        // Drop the icon behind the tray's back, as an Explorer restart does.
        // SAFETY: plain data; zero is its empty value.
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = 1;
        unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
        let dropped = installed_tray();
        println!("after the shell dropped it: {dropped:?}");
        if dropped.is_some() {
            return Err("the shell still places the icon after NIM_DELETE".into());
        }
        let name = wide("TaskbarCreated");
        let taskbar_created = unsafe { RegisterWindowMessageW(name.as_ptr()) };
        unsafe { SendMessageW(hwnd, taskbar_created, 0, 0) };
        let readded = installed_tray();
        println!("after TaskbarCreated: {readded:?}");
        if readded.is_none() {
            return Err("TaskbarCreated did not add the icon again".into());
        }

        // Theme and DPI changes rebuild the icon in place.
        let theme = wide("ImmersiveColorSet");
        unsafe { SendMessageW(hwnd, WM_SETTINGCHANGE, 0, theme.as_ptr() as isize) };
        unsafe { SendMessageW(hwnd, WM_DPICHANGED, (144 << 16) | 144, 0) };
        let refreshed = installed_tray();
        println!("after theme and DPI changes: {refreshed:?}");
        if refreshed.is_none() {
            return Err("a theme or DPI refresh lost the icon".into());
        }
        Ok(())
    }
}
