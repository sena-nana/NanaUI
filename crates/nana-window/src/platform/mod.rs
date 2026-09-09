#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) use fallback::{apply, clear, set_application_icon_png};
#[cfg(target_os = "macos")]
pub(crate) use macos::{
    apply, clear, describe_configured_panel, install_menu_bar, installed_menu_bar,
    open_file_dialog, set_application_icon_png,
};
#[cfg(target_os = "windows")]
pub(crate) use windows::{
    apply, clear, install_menu_bar, open_file_dialog, set_application_icon_png,
};
