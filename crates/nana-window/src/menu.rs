//! Native application menu bar.
//!
//! The menu bar is the one piece of desktop chrome that cannot be drawn in the
//! UI tree: on macOS it belongs to the application, not the window, and lives
//! in the system menu bar. So it goes here, beside the other platform-owned
//! window work, and is described with a plain model the application builds.
//!
//! Controls never reach this. The application declares the menu and drains the
//! ids of whatever the user chose; what an id means stays application business,
//! the same shape as `ActionRegistry`.
//!
//! Platform support is deliberately uneven and says so:
//! macOS is the primary target (a Mac app without a menu bar is broken),
//! Windows gets an in-window `HMENU`, and other platforms are a no-op —
//! [`menu_bar_support`] reports which you got.

use std::sync::{Mutex, OnceLock};

pub use nana_ui_core::{Menu, MenuBar, MenuEntry, MenuShortcut};

/// How much of the menu bar the running platform provides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuBarSupport {
    /// System menu bar owned by the application (macOS).
    System,
    /// Menu strip inside the window (Windows).
    InWindow,
    /// Nothing was installed. Put the commands in the UI instead.
    Unavailable,
}

/// What [`install_menu_bar`] will do on this platform.
pub const fn menu_bar_support() -> MenuBarSupport {
    #[cfg(target_os = "macos")]
    {
        MenuBarSupport::System
    }
    #[cfg(target_os = "windows")]
    {
        MenuBarSupport::InWindow
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        MenuBarSupport::Unavailable
    }
}

/// Ids the user chose since the last drain.
fn activations() -> &'static Mutex<Vec<u32>> {
    static ACTIVATIONS: OnceLock<Mutex<Vec<u32>>> = OnceLock::new();
    ACTIVATIONS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Records a chosen id. Called from the platform's menu callback.
pub(crate) fn push_activation(id: u32) {
    if let Ok(mut queue) = activations().lock() {
        queue.push(id);
    }
}

/// Menu ids the user chose since the last call, in order.
///
/// Drain this once a frame and route the ids the way you route any other
/// command. Never empty-loops: an untouched menu yields an empty vec.
pub fn take_menu_activations() -> Vec<u32> {
    activations()
        .lock()
        .map(|mut queue| std::mem::take(&mut *queue))
        .unwrap_or_default()
}

/// Installs `bar` as the application's menu bar, without a window.
///
/// macOS only: there the menu belongs to the application, so no window is
/// involved. Every other platform attaches the menu to a window and reports
/// [`MenuBarSupport::Unavailable`] here — use [`install_menu_bar`].
pub fn install_application_menu_bar(bar: &MenuBar) -> MenuBarSupport {
    let _ = bar;
    #[cfg(target_os = "macos")]
    {
        crate::platform::install_menu_bar(bar);
        MenuBarSupport::System
    }
    #[cfg(not(target_os = "macos"))]
    {
        MenuBarSupport::Unavailable
    }
}

/// Titles of the installed menu bar, top level first, each with its entries.
///
/// Reads back what the platform actually holds, so a host can verify the menu
/// it asked for is the menu that exists. Returns `None` where the platform
/// cannot be queried.
pub fn installed_menu_bar() -> Option<Vec<(String, Vec<String>)>> {
    #[cfg(target_os = "macos")]
    {
        crate::platform::installed_menu_bar()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Installs `bar` as the application's menu bar.
///
/// Call it again with a new model to replace the whole bar; there is no
/// incremental item API, because rebuilding a menu is cheap and diffing one
/// across three platforms is not.
///
/// `window` is only used where the menu belongs to a window (Windows). On
/// macOS the bar is the application's and the handle is ignored.
pub fn install_menu_bar<W: raw_window_handle::HasWindowHandle + ?Sized>(
    window: &W,
    bar: &MenuBar,
) -> MenuBarSupport {
    let _ = (window, bar);
    #[cfg(target_os = "macos")]
    {
        crate::platform::install_menu_bar(bar);
        MenuBarSupport::System
    }
    #[cfg(target_os = "windows")]
    {
        crate::platform::install_menu_bar(window, bar);
        MenuBarSupport::InWindow
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        MenuBarSupport::Unavailable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activations_drain_in_order_and_leave_the_queue_empty() {
        // The queue is process-global; drain whatever a parallel test left.
        let _ = take_menu_activations();
        push_activation(7);
        push_activation(9);
        assert_eq!(take_menu_activations(), vec![7, 9]);
        assert!(take_menu_activations().is_empty());
    }

    #[test]
    fn builders_only_touch_items() {
        let item = MenuEntry::item(1, "Save")
            .shortcut(MenuShortcut::primary("s"))
            .enabled(false)
            .checked(true);
        let MenuEntry::Item {
            id,
            shortcut,
            enabled,
            checked,
            ..
        } = item
        else {
            panic!("expected an item");
        };
        assert_eq!(id, 1);
        assert_eq!(shortcut, Some(MenuShortcut::primary("s")));
        assert!(!enabled);
        assert!(checked);

        // A separator ignores item-only builders rather than panicking.
        assert_eq!(
            MenuEntry::Separator.enabled(false).checked(true),
            MenuEntry::Separator
        );
    }
}
