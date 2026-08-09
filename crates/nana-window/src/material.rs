use raw_window_handle::HasWindowHandle;

use crate::platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FallbackColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl FallbackColor {
    pub const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    #[cfg(target_os = "windows")]
    pub(crate) const fn tuple(self) -> window_vibrancy::Color {
        (self.red, self.green, self.blue, self.alpha)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialEffect {
    Solid,
    Transparent,
    Vibrancy,
    Mica,
    Acrylic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialFallback {
    PlatformDoesNotProvideNativeMaterial,
    NativeMaterialUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaterialOutcome {
    pub effect: MaterialEffect,
    pub fallback: Option<MaterialFallback>,
}

impl MaterialOutcome {
    pub const fn native(effect: MaterialEffect) -> Self {
        Self {
            effect,
            fallback: None,
        }
    }

    pub const fn solid(fallback: MaterialFallback) -> Self {
        Self {
            effect: MaterialEffect::Solid,
            fallback: Some(fallback),
        }
    }

    pub const fn transparent() -> Self {
        Self {
            effect: MaterialEffect::Transparent,
            fallback: None,
        }
    }

    pub const fn is_native(self) -> bool {
        matches!(
            self.effect,
            MaterialEffect::Vibrancy | MaterialEffect::Mica | MaterialEffect::Acrylic
        )
    }
}

pub fn apply_system_material<W: HasWindowHandle + ?Sized>(
    window: &W,
    appearance: Appearance,
    fallback: FallbackColor,
) -> MaterialOutcome {
    platform::apply(window, appearance, fallback)
}

/// Applies the platform material that is safe for a host-owned GPU surface.
///
/// AppKit visual-effect subviews composite above the content view's CAMetalLayer,
/// so macOS hosted renderers must use their opaque surface fallback until the
/// window owns a separate content view behind the WGPU layer.
pub fn apply_hosted_system_material<W: HasWindowHandle + ?Sized>(
    window: &W,
    appearance: Appearance,
    fallback: FallbackColor,
) -> MaterialOutcome {
    #[cfg(target_os = "macos")]
    {
        let _ = (appearance, fallback);
        platform::clear(window);
        MaterialOutcome::solid(MaterialFallback::NativeMaterialUnavailable)
    }
    #[cfg(not(target_os = "macos"))]
    {
        platform::apply(window, appearance, fallback)
    }
}

pub fn clear_system_material<W: HasWindowHandle + ?Sized>(window: &W) {
    platform::clear(window);
}

#[cfg(test)]
mod tests {
    use super::{MaterialEffect, MaterialFallback, MaterialOutcome};

    #[test]
    fn transparent_is_an_explicit_non_native_outcome() {
        let outcome = MaterialOutcome::transparent();

        assert_eq!(outcome.effect, MaterialEffect::Transparent);
        assert_eq!(outcome.fallback, None);
        assert!(!outcome.is_native());
    }

    #[test]
    fn only_platform_materials_are_native() {
        assert!(!MaterialOutcome::solid(MaterialFallback::NativeMaterialUnavailable).is_native());
        assert!(MaterialOutcome::native(MaterialEffect::Acrylic).is_native());
    }
}
