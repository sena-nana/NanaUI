//! Platform-owned native window support for Nana applications.

mod chrome;
mod material;
mod platform;

pub use chrome::drag_custom_title_bar;
pub use chrome::prepare_client_chrome;
pub use chrome::prepare_custom_title_bar;
pub use material::{
    Appearance, FallbackColor, MaterialEffect, MaterialFallback, MaterialOutcome,
    PlatformMaterialSupport, apply_hosted_system_material, apply_system_material,
    clear_system_material, platform_material_support,
};
