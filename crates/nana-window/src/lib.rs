//! Platform-owned window material support for Nana applications.

mod material;
mod platform;

pub use material::{
    Appearance, FallbackColor, MaterialEffect, MaterialFallback, MaterialOutcome,
    apply_system_material, clear_system_material,
};
