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

impl MaterialEffect {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Solid => "实色",
            Self::Transparent => "透明",
            Self::Vibrancy => "Vibrancy",
            Self::Mica => "Mica",
            Self::Acrylic => "Acrylic",
        }
    }

    pub const fn wants_transparent_surface(self) -> bool {
        !matches!(self, Self::Solid)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialFallback {
    PlatformDoesNotProvideNativeMaterial,
    NativeMaterialUnavailable,
}

impl MaterialFallback {
    pub const fn label(self) -> &'static str {
        match self {
            Self::PlatformDoesNotProvideNativeMaterial => "当前设备不支持透明窗口效果",
            Self::NativeMaterialUnavailable => "透明效果不可用，已使用实色背景",
        }
    }
}

/// Platform-level native material capability, without requiring a window handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformMaterialSupport {
    /// macOS: Vibrancy / UnderWindowBackground.
    Vibrancy,
    /// Windows: Mica and Acrylic APIs exist; the caller must request one.
    MicaAcrylic,
    /// No native material API is invoked.
    None,
}

impl PlatformMaterialSupport {
    pub const fn hint(self) -> &'static str {
        match self {
            Self::Vibrancy => "可申请 Vibrancy；未申请或调用失败时使用实色背景。",
            Self::MicaAcrylic => "可申请 Mica 或 Acrylic；未申请或调用失败时使用实色背景。",
            Self::None => "当前设备使用实色窗口背景。",
        }
    }
}

/// Returns the compile-time material capability of the current target OS.
pub const fn platform_material_support() -> PlatformMaterialSupport {
    if cfg!(target_os = "macos") {
        PlatformMaterialSupport::Vibrancy
    } else if cfg!(target_os = "windows") {
        PlatformMaterialSupport::MicaAcrylic
    } else {
        PlatformMaterialSupport::None
    }
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

    /// User explicitly chose an opaque window background.
    pub const fn chosen_solid() -> Self {
        Self {
            effect: MaterialEffect::Solid,
            fallback: None,
        }
    }

    pub const fn is_native(self) -> bool {
        matches!(
            self.effect,
            MaterialEffect::Vibrancy | MaterialEffect::Mica | MaterialEffect::Acrylic
        )
    }

    pub const fn wants_transparent_surface(self) -> bool {
        self.effect.wants_transparent_surface()
    }

    pub fn status_label(self) -> String {
        match self.fallback {
            Some(reason) => format!("{}（{}）", self.effect.label(), reason.label()),
            None if self.is_native() => "透明效果已启用".to_string(),
            None => "实色背景".to_string(),
        }
    }
}

pub fn apply_system_material<W: HasWindowHandle + ?Sized>(
    window: &W,
    requested: MaterialEffect,
    appearance: Appearance,
    fallback: FallbackColor,
) -> MaterialOutcome {
    platform::apply(window, requested, appearance, fallback)
}

/// Applies the requested effect on a host-owned GPU surface.
///
/// Scene GPU's CAMetalLayer covers AppKit visual-effect views, so a hosted macOS
/// Vibrancy request reports [`MaterialFallback::NativeMaterialUnavailable`].
pub fn apply_hosted_system_material<W: HasWindowHandle + ?Sized>(
    window: &W,
    requested: MaterialEffect,
    appearance: Appearance,
    fallback: FallbackColor,
) -> MaterialOutcome {
    #[cfg(target_os = "macos")]
    {
        match requested {
            MaterialEffect::Vibrancy => {
                platform::clear(window);
                MaterialOutcome::solid(MaterialFallback::NativeMaterialUnavailable)
            }
            MaterialEffect::Mica | MaterialEffect::Acrylic => {
                platform::clear(window);
                MaterialOutcome::solid(MaterialFallback::PlatformDoesNotProvideNativeMaterial)
            }
            _ => platform::apply(window, requested, appearance, fallback),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        platform::apply(window, requested, appearance, fallback)
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

    #[test]
    fn transparent_surface_is_explicit_and_not_native_blur() {
        assert!(MaterialOutcome::transparent().wants_transparent_surface());
        assert!(!MaterialOutcome::transparent().is_native());
        assert!(!MaterialOutcome::chosen_solid().wants_transparent_surface());
    }
}
