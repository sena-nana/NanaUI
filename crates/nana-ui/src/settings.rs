//! Settings model re-exports.
//!
//! Leaf chrome (`SettingsRow`, `SettingsCard`) is implemented in Runtime.

pub use nana_ui_core::AppearanceEvent;
pub use nana_ui_core::settings::{
    AppearanceSettings, BackdropTarget, SettingsError, SettingsModel, SettingsState, SettingsTab,
    SettingsTabId, WindowMaterialMode,
};
use nana_window::MaterialEffect;

pub fn window_material_effect(mode: WindowMaterialMode) -> MaterialEffect {
    match mode {
        WindowMaterialMode::Solid => MaterialEffect::Solid,
        WindowMaterialMode::Translucent => MaterialEffect::Transparent,
        WindowMaterialMode::Vibrancy => MaterialEffect::Vibrancy,
        WindowMaterialMode::Mica => MaterialEffect::Mica,
        WindowMaterialMode::Acrylic => MaterialEffect::Acrylic,
    }
}
