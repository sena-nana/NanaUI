//! Platform-owned native window support for Nana applications.

mod browser;
mod chrome;
mod pointer;
pub use browser::{
    BrowserCommand, BrowserCompletion, BrowserEvent, BrowserPolicy, BrowserRect, BrowserState,
    NativeBrowser,
};
mod file_dialog;
mod material;
mod menu;
mod motion_preference;
mod platform;
mod size_move;

pub use chrome::FrameResizeEdge;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub use chrome::LiveFrameMove;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub use chrome::LiveFrameResize;
pub use chrome::drag_custom_title_bar;
pub use chrome::native_live_resize_active;
pub use chrome::place_native_window_controls;
pub use chrome::prepare_client_chrome;
pub use chrome::prepare_custom_title_bar;
pub use chrome::resize_custom_frame;
pub use chrome::set_native_window_controls_visible;
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
pub use motion_preference::{system_reduced_motion, take_reduced_motion_change};
pub use pointer::pointer_in_client_area;
pub use size_move::LiveSizeMove;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipTaskbarError {
    /// The platform has no per-window taskbar entry to hide.
    Unsupported(String),
    /// The platform refused or failed the request; the entry is unchanged.
    Failed(String),
}

impl std::fmt::Display for SkipTaskbarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(reason) => write!(f, "skip taskbar unsupported: {reason}"),
            Self::Failed(reason) => write!(f, "skip taskbar failed: {reason}"),
        }
    }
}

impl std::error::Error for SkipTaskbarError {}

/// Show or hide the window's taskbar entry.
///
/// On Windows a hidden entry stays hidden when the window is shown again or
/// Explorer restarts. Other platforms return `Unsupported`.
pub fn set_skip_taskbar<W: raw_window_handle::HasWindowHandle + ?Sized>(
    window: &W,
    skip: bool,
) -> Result<(), SkipTaskbarError> {
    #[cfg(target_os = "windows")]
    {
        platform::set_skip_taskbar(window, skip)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (window, skip);
        let reason = if cfg!(target_os = "macos") {
            "the Dock shows the application, not individual windows"
        } else {
            "this backend has no per-window taskbar entry"
        };
        Err(SkipTaskbarError::Unsupported(reason.into()))
    }
}

/// Show a window without making it key or activating the application.
///
/// winit's `set_visible(true)` makes a macOS window key, so a window shown
/// after creation would ignore `WindowDescriptor::focus_on_show = false`.
/// Returns whether the window was shown here; other platforms return false
/// and the caller shows it through winit.
pub fn show_without_activation<W: raw_window_handle::HasWindowHandle + ?Sized>(window: &W) -> bool {
    platform::show_without_activation(window)
}

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
