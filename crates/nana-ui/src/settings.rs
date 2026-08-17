//! Settings model re-exports.
//!
//! Leaf chrome (`SettingsRow`, `SettingsCard`) is implemented in Runtime.
//! Iced `settings_page` / `settings_sidebar` composers were removed.

pub use nana_ui_core::AppearanceEvent;
pub use nana_ui_core::settings::{
    AppearanceSettings, BackdropTarget, SettingsError, SettingsModel, SettingsState, SettingsTab,
    SettingsTabId, WindowMaterialMode,
};
