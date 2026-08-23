//! Platform-owned native window support for Nana applications.

mod chrome;
mod material;
mod platform;

pub use chrome::drag_custom_title_bar;
pub use chrome::prepare_client_chrome;
pub use chrome::prepare_custom_title_bar;
pub use chrome::suppress_system_caption;
pub use material::{
    Appearance, FallbackColor, MaterialEffect, MaterialFallback, MaterialOutcome,
    PlatformMaterialSupport, apply_hosted_system_material, apply_system_material,
    clear_system_material, hosted_platform_material_support, platform_material_support,
};

/// macOS Dock / application icon from PNG bytes. No-op on other platforms.
///
/// winit's window icon is ignored on macOS; this talks to `NSApplication`.
pub fn set_application_icon_png(png: &[u8]) {
    platform::set_application_icon_png(png);
}
