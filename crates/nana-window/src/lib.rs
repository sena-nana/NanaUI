//! Platform-owned native window support for Nana applications.

mod browser;
mod chrome;
pub use browser::{
    BrowserCommand, BrowserCompletion, BrowserEvent, BrowserPolicy, BrowserRect, BrowserState,
    NativeBrowser,
};
mod file_dialog;
mod material;
mod menu;
mod platform;
mod size_move;

pub use chrome::FrameResizeEdge;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub use chrome::LiveFrameResize;
pub use chrome::drag_custom_title_bar;
pub use chrome::native_live_resize_active;
pub use chrome::prepare_client_chrome;
pub use chrome::prepare_custom_title_bar;
pub use chrome::resize_custom_frame;
pub use chrome::set_present_transaction;
pub use chrome::suppress_system_caption;
pub use file_dialog::{
    FileDialogError, FileDialogHandle, FileDialogKind, FileDialogRequest, FileDialogResult,
    FileDialogSupport, FileFilter, describe_configured_dialog, file_dialog_support,
    open_file_dialog,
};
pub use material::{
    Appearance, FallbackColor, MaterialEffect, MaterialFallback, MaterialOutcome,
    PlatformMaterialSupport, apply_hosted_system_material, apply_system_material,
    clear_system_material, hosted_platform_material_support, platform_material_support,
};
pub use menu::{
    Menu, MenuBar, MenuBarSupport, MenuEntry, MenuShortcut, install_application_menu_bar,
    install_menu_bar, installed_menu_bar, menu_bar_support, take_menu_activations,
};
pub use size_move::LiveSizeMove;

/// macOS Dock / application icon from PNG bytes. No-op on other platforms.
///
/// winit's window icon is ignored on macOS; this talks to `NSApplication`.
pub fn set_application_icon_png(png: &[u8]) {
    platform::set_application_icon_png(png);
}

/// Physical work area (excluding taskbars) for the display containing the point.
/// Platforms without this query return None so hosts can use monitor bounds.
pub fn display_work_area(point: (i32, i32)) -> Option<((i32, i32), (u32, u32))> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
        };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..unsafe { std::mem::zeroed() }
        };
        unsafe {
            let monitor = MonitorFromPoint(
                POINT {
                    x: point.0,
                    y: point.1,
                },
                MONITOR_DEFAULTTONEAREST,
            );
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return None;
            }
        }
        let rect = info.rcWork;
        Some((
            (rect.left, rect.top),
            (
                (rect.right - rect.left) as u32,
                (rect.bottom - rect.top) as u32,
            ),
        ))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = point;
        None
    }
}
