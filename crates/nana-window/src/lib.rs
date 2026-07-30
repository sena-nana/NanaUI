//! Platform-owned native window support for Nana applications.

mod chrome;
mod material;
mod platform;

#[cfg(target_os = "macos")]
pub use chrome::drag_custom_title_bar;
pub use chrome::prepare_custom_title_bar;
pub use material::{
    Appearance, FallbackColor, MaterialEffect, MaterialFallback, MaterialOutcome,
    apply_system_material, clear_system_material,
};
